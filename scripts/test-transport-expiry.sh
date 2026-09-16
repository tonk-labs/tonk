#!/usr/bin/env bash
# Exercise the unreleased Dialog transport expiry patch without rewriting this checkout.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
dialog_revision=4c16de9e345d2b2d888d1008c5d3f0ca990c4807
patch_file="$repo_root/patches/dialog-transport-expiry.patch"
prepare_only=false
if [[ "${1:-}" == --prepare-only ]]; then
  prepare_only=true
  shift
fi
if [[ "${1:-}" == -- ]]; then shift; fi
if [[ "$prepare_only" == true && $# -ne 0 ]]; then
  echo "--prepare-only does not accept Cargo arguments" >&2
  exit 2
fi
if [[ ! -s "$patch_file" ]]; then
  echo "missing transport expiry patch: $patch_file" >&2
  exit 2
fi
for required in git rsync python3 cargo; do
  command -v "$required" >/dev/null || { echo "missing tool: $required" >&2; exit 2; }
done
python3 -c 'import tomllib' || {
  echo "Python 3.11 or newer is required" >&2
  exit 2
}
if [[ -L "$repo_root/Cargo.lock" || -L "$repo_root/Cargo.toml" ]]; then
  echo "Cargo.lock and Cargo.toml must be regular files for an isolated snapshot" >&2
  exit 2
fi
if [[ -n "${CARGO_TARGET_DIR:-}" && "$CARGO_TARGET_DIR" != /* ]]; then
  echo "CARGO_TARGET_DIR must be absolute when sharing build artifacts" >&2
  exit 2
fi

dialog_source=${DIALOG_SOURCE:-}
if [[ -z "$dialog_source" ]]; then
  if [[ -d "$repo_root/.wt/dialog-connection-authorizer" ]]; then
    dialog_source="$repo_root/.wt/dialog-connection-authorizer"
  else
    dialog_source=https://github.com/dialog-db/dialog-db.git
  fi
fi
run_root=$(mktemp -d "${TMPDIR:-/tmp}/tonk-transport-expiry.XXXXXX")
# Retain failures as well as successful runs for diagnosis. Never clean the user's
# checkout or restore its lockfile: all Cargo mutations happen in this snapshot.
printf 'sandbox=%s\n' "$run_root"
trap 'printf "retained sandbox=%s\n" "$run_root" >&2' EXIT
mkdir -p "$run_root/tonk"
rsync -a \
  --exclude='.git' --exclude='.wt' --exclude='target' \
  --exclude='.direnv' --exclude='node_modules' --exclude='.tonk' \
  --exclude='.cache' --exclude='result' --exclude='result-*' \
  "$repo_root/" "$run_root/tonk/"
git clone --quiet --no-hardlinks --no-checkout "$dialog_source" "$run_root/dialog"
git -C "$run_root/dialog" checkout --quiet --detach "$dialog_revision"
git -C "$run_root/dialog" apply --check "$run_root/tonk/patches/dialog-transport-expiry.patch"
git -C "$run_root/dialog" apply "$run_root/tonk/patches/dialog-transport-expiry.patch"
python3 "$run_root/tonk/scripts/sync-dialog-transport-vendor.py" "$run_root/dialog" --check

config_file="$run_root/dialog-patch.toml"
python3 - "$run_root/tonk" "$run_root/dialog" "$config_file" "$dialog_revision" <<'PY'
import json
import pathlib
import sys
import tomllib

tonk, dialog, output = map(pathlib.Path, sys.argv[1:4])
revision = sys.argv[4]
repository = "https://github.com/dialog-db/dialog-db.git"
lock = tomllib.loads((tonk / "Cargo.lock").read_text())
spike = tonk / "spikes/transport-expiry.rs"
if not spike.is_file():
    raise SystemExit(f"missing consumer spike: {spike}")
consumer_manifest = tonk / "rust/tonk-access-service/Cargo.toml"
consumer_text = consumer_manifest.read_text()
consumer = tomllib.loads(consumer_text)
if any(test.get("name") == "transport_expiry_spike" for test in consumer.get("test", [])):
    raise SystemExit("transport_expiry_spike already registered; reassess harness injection")
# Register the transport consumer only in the isolated copy. The ordinary
# checkout uses the reproduced single-crate vendor through its Cargo patch.
consumer_manifest.write_text(consumer_text + '''
[[test]]
name = "transport_expiry_spike"
path = "../../spikes/transport-expiry.rs"
required-features = ["helpers"]
''')
packages = {}
for manifest in dialog.rglob("Cargo.toml"):
    if any(part in (".git", "target") for part in manifest.parts):
        continue
    package = tomllib.loads(manifest.read_text()).get("package", {})
    if "name" in package:
        name = package["name"]
        if name in packages:
            raise SystemExit(f"duplicate Dialog package: {name}")
        packages[name] = manifest.parent

overrides = {}
for package in lock["package"]:
    source = package.get("source", "")
    if source.startswith(f"git+{repository}?"):
        if source.rsplit("#", 1)[-1] != revision:
            raise SystemExit(f"unexpected Dialog revision for {package['name']}: {source}")
        name = package["name"]
        if name not in packages:
            raise SystemExit(f"Dialog package missing from checkout: {name}")
        overrides[name] = packages[name]
if not overrides:
    raise SystemExit("no pinned Dialog packages found in Cargo.lock")
# Match the whole source, including transitive helpers/macros such as wbg-pool,
# rather than just workspace dependency names. Partial overrides duplicate types.
lines = [f"[patch.{json.dumps(repository)}]"]
for name, directory in sorted(overrides.items()):
    lines.append(f"{json.dumps(name)} = {{ path = {json.dumps(str(directory))} }}")
output.write_text("\n".join(lines) + "\n")
print(f"patched Dialog packages={len(overrides)}")
PY

printf 'tonk snapshot=%s\ndialog checkout=%s\ncargo config=%s\n' \
  "$run_root/tonk" "$run_root/dialog" "$config_file"
if [[ "$prepare_only" == true ]]; then
  printf 'prepared; run Cargo from the snapshot with --config %s\n' "$config_file"
  exit 0
fi
if [[ $# -eq 0 ]]; then
  set -- test -p tonk-access-service --features helpers --test transport_expiry_spike
fi
cd "$run_root/tonk"
cargo --config "$config_file" "$@"

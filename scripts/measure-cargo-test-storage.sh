#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  echo "usage: CARGO_TARGET_DIR=/absolute/empty/path $0" >&2
  exit 2
fi

case "$CARGO_TARGET_DIR" in
/*) ;;
*)
  echo "measure-cargo-test-storage: CARGO_TARGET_DIR must be absolute" >&2
  exit 2
  ;;
esac

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
normal_target="$repo_root/target"
if [[ "$CARGO_TARGET_DIR" == "$repo_root" || "$CARGO_TARGET_DIR" == "$normal_target" ]]; then
  echo "measure-cargo-test-storage: refusing repository path: $CARGO_TARGET_DIR" >&2
  exit 2
fi

mkdir -p "$CARGO_TARGET_DIR"
if find "$CARGO_TARGET_DIR" -mindepth 1 -print -quit | grep -q .; then
  echo "measure-cargo-test-storage: target directory must be empty: $CARGO_TARGET_DIR" >&2
  exit 2
fi

bytes_below() {
  local path=$1
  local kib=0
  if [[ -e "$path" ]]; then
    kib=$(du -sk "$path" | awk '{print $1}')
  fi
  printf '%s' "$((kib * 1024))"
}

run_builds() {
  cargo test --locked -p tonk-ui --features integration-tests --no-run
  cargo test --locked --target wasm32-unknown-unknown -p tonk-ui --no-run
}

cd "$repo_root"
run_builds
first_total=$(bytes_below "$CARGO_TARGET_DIR")
run_builds
second_total=$(bytes_below "$CARGO_TARGET_DIR")

printf 'target_dir=%s\n' "$CARGO_TARGET_DIR"
printf 'total_bytes=%s\n' "$second_total"
printf 'first_pass_total_bytes=%s\n' "$first_total"
printf 'repeat_growth_bytes=%s\n' "$((second_total - first_total))"
printf 'debug_incremental_bytes=%s\n' "$(bytes_below "$CARGO_TARGET_DIR/debug/incremental")"
printf 'debug_deps_bytes=%s\n' "$(bytes_below "$CARGO_TARGET_DIR/debug/deps")"
printf 'wasm32_unknown_unknown_bytes=%s\n' "$(bytes_below "$CARGO_TARGET_DIR/wasm32-unknown-unknown")"
printf 'rust_analyzer_bytes=%s\n' "$(bytes_below "$CARGO_TARGET_DIR/rust-analyzer")"

#!/bin/sh
# Drive `tonk update` end-to-end against a fake release served from
# localhost. Nothing here touches your real tonk, your real update
# state, or the network.
#
#   sh scripts/selfupdate-lab.sh [path-to-tonk-binary]
#
# Defaults to target/debug/tonk. Everything happens in a temp dir that
# is removed on exit.
#
# The seams this leans on are the same ones tests/update.rs uses:
#   TONK_UPDATE_ENDPOINT  the release base URL (default: github.com/...)
#   TONK_UPDATE_STATE     the dir holding install.json + update.json
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TONK="${1:-$ROOT/target/debug/tonk}"
[ -x "$TONK" ] || { echo "no tonk binary at $TONK (cargo build -p tonk-cli)" >&2; exit 1; }

case "$(uname -s)/$(uname -m)" in
  Darwin/arm64) PLATFORM=macos-arm64 ;;
  Linux/x86_64) PLATFORM=linux-x86_64 ;;
  *) echo "no published platform slug for $(uname -s)/$(uname -m)" >&2; exit 1 ;;
esac

LAB="$(mktemp -d)"
trap 'kill ${SRV:-0} 2>/dev/null || true; rm -rf "$LAB"' EXIT

# This lab records a staging install and mirrors the staging release
# layout so the CLI's receipt-based URL selection is exercised.
REL="$LAB/releases/download/tonk-staging"
mkdir -p "$REL" "$LAB/bin" "$LAB/state" "$LAB/stage"

# The copy we let it overwrite. A real install.sh install is a plain
# file on PATH, which is exactly what this is — and it is under neither
# /nix/store nor node_modules, so the foreign-install guard lets it
# through.
cp "$TONK" "$LAB/bin/tonk"
INSTALL_DIR="$(cd "$LAB/bin" && pwd -P)"

# The "new release": a stand-in that reports a newer version. It only
# has to pass the `--version` smoke test, and using an obviously
# different version makes the swap impossible to fake.
printf '#!/bin/sh\necho "tonk 9.9.9 (the fake new release)"\n' > "$LAB/stage/tonk"
chmod +x "$LAB/stage/tonk"
ASSET="tonk-$PLATFORM.tar.gz"
tar -czf "$REL/$ASSET" -C "$LAB/stage" tonk

# checksums.txt in sha256sum's format — the integrity gate refuses to
# install anything that does not match.
( cd "$REL" && { command -v sha256sum >/dev/null 2>&1 && sha256sum "$ASSET" || shasum -a 256 "$ASSET"; } > checksums.txt )

COMMIT=deadbeefcafe1234567890
cat > "$REL/manifest.json" <<EOF
{
  "version": "9.9.9",
  "commit": "$COMMIT",
  "channel": "staging",
  "built_at": "2026-07-16T00:00:00Z"
}
EOF

# Model the receipt install.sh writes for this exact copy. The commit
# intentionally differs from the fake release so the update downloads
# and swaps the binary instead of reporting "already current".
cat > "$LAB/state/install.json" <<EOF
{
  "channel": "staging",
  "version": "0.0.0",
  "commit": "before-lab",
  "install_dir": "$INSTALL_DIR",
  "installed_at": "2026-07-16T00:00:00Z"
}
EOF

PORT=8973
python3 -m http.server "$PORT" --bind 127.0.0.1 -d "$LAB" >/dev/null 2>&1 &
SRV=$!
until curl -fsS "http://127.0.0.1:$PORT/releases/download/tonk-staging/manifest.json" >/dev/null 2>&1; do sleep 0.1; done

export TONK_UPDATE_ENDPOINT="http://127.0.0.1:$PORT/releases"
export TONK_UPDATE_STATE="$LAB/state"
export TONK_TELEMETRY_STATE="$LAB/state"
# Prove ambient installer input cannot override this copy's receipt.
export TONK_CHANNEL=stable
unset TONK_NO_UPDATE_CHECK CI 2>/dev/null || true

say() { printf '\n\033[1m%s\033[0m\n' "$1"; }

say "0. what we start from"
"$LAB/bin/tonk" --version

say "1. the nag — any command, on stderr, cache-driven"
echo "   (stdout and stderr split, so you can see which side it lands on)"
"$LAB/bin/tonk" telemetry >"$LAB/out" 2>"$LAB/err" || true
echo "   stdout: $(cat "$LAB/out")"
echo "   stderr: $(cat "$LAB/err")"

say "2. the nag does not repeat within 24h"
"$LAB/bin/tonk" telemetry >/dev/null 2>"$LAB/err2" || true
[ -s "$LAB/err2" ] && echo "   stderr: $(cat "$LAB/err2")" || echo "   stderr: (silent — already nagged today)"

say "3. tonk update — download, verify, smoke-test, swap"
"$LAB/bin/tonk" update

say "4. proof the swap really happened"
"$LAB/bin/tonk" --version
echo "   receipt:"
sed 's/^/     /' "$LAB/state/install.json"

say "5. already current — restore a real tonk, re-run against the same release"
cp "$TONK" "$LAB/bin/tonk"
"$LAB/bin/tonk" update

say "6. the integrity gate — corrupt the archive, keep the checksum"
# The commit has to move too. Otherwise the receipt from step 3 still
# matches and `update` short-circuits to "already current" without ever
# downloading — the gate would look like it passed while never running.
printf 'not a tarball' > "$REL/$ASSET"
sed 's/"commit": "[^"]*"/"commit": "feedface0000000000000"/' "$REL/manifest.json" > "$REL/m.tmp"
mv "$REL/m.tmp" "$REL/manifest.json"
if "$LAB/bin/tonk" update; then
  echo "   !!! it installed a corrupt archive"
else
  echo "   ^ refused. and the binary it was about to replace is untouched:"
fi
"$LAB/bin/tonk" --version

say "done — $LAB is about to be removed; your real tonk was never touched"

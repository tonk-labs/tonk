#!/bin/sh
# Build what spaces that run a kernel need, under `/_host/` (see
# `plan/space-kernel.md`): the embedder a space's bootloader loads, built for
# the browser and bound with wasm-bindgen, and the kernel this deployment
# installs into a space it creates, which `tonk-components` builds for
# `wasm32-wasip3` and names. Cargo builds each only when its sources changed.
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ROOT="$SCRIPT_DIR/../../.."
HOST_DIR="${TRUNK_STAGING_DIR:?TRUNK_STAGING_DIR not set}/_host"
PROFILE="${TRUNK_PROFILE:-debug}"
case "$PROFILE" in
  release) FLAGS="--release"; OUT=release ;;
  *) FLAGS=""; OUT=debug ;;
esac
cd "$ROOT"
cargo build -p tonk-kernel-host --lib --target wasm32-unknown-unknown $FLAGS
rm -rf "$HOST_DIR"
mkdir -p "$HOST_DIR"
wasm-bindgen --target no-modules --no-typescript --out-dir "$HOST_DIR" --out-name embedder \
  "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/$OUT/tonk_kernel_host.wasm"
cargo run -q -p tonk-components --bin write-components -- "$HOST_DIR"
# The profile's shell names the persona this deployment ships, so that it can
# take a profile a worker from before kernels serves over to it: that worker
# does not answer `/_host/kernel.json`.
PERSONA=$(sed -n 's/.*"persona":"\([a-z2-7]*\)".*/\1/p' "$HOST_DIR/kernel.json")
SHELL="${TRUNK_STAGING_DIR}/profile.html"
if [ -f "$SHELL" ]; then
  sed -i.bak "s|var SHIPPED_PERSONA = null;|var SHIPPED_PERSONA = \"$PERSONA\";|" "$SHELL"
  rm -f "$SHELL.bak"
  grep -q "var SHIPPED_PERSONA = \"$PERSONA\";" "$SHELL" || {
    echo "build-kernel-host: the profile's shell names no persona" >&2
    exit 1
  }
fi

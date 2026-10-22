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
  "target/wasm32-unknown-unknown/$OUT/tonk_kernel_host.wasm"
cargo run -q -p tonk-components --bin write-components -- "$HOST_DIR"

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
# The kernels offer connections to a space where the index builds the app
# with them.
COMPONENT_FEATURES=""
if grep -q 'data-bin="ui"[^>]*data-cargo-features="[^"]*connection-invites' "${TRUNK_HTML_FILE:-/dev/null}" 2>/dev/null; then
  COMPONENT_FEATURES="--features connection-invites"
fi
# The tool runs here, on the machine building, whatever target the build of
# the app names (Nix names the browser's for the whole derivation).
HOST_TARGET=$(rustc -vV | sed -n 's/^host: //p')
cargo run -q -p tonk-components $COMPONENT_FEATURES --target "$HOST_TARGET" --bin write-components -- "$HOST_DIR"
# A site keeps the kernel it pins, and a dist is built from nothing each
# time. Where a store of kernels is named, the ones built here are kept in
# it, the store is held to its newest few, and the dist serves every kernel
# the store still has.
if [ -n "${TONK_KERNEL_STORE:-}" ]; then
  KEEP=${TONK_KERNEL_KEEP:-50}
  mkdir -p "$TONK_KERNEL_STORE"
  for BUILT in "$HOST_DIR"/kernel/*.wasm; do
    NAME=${BUILT##*/}
    [ -f "$TONK_KERNEL_STORE/$NAME" ] || cp "$BUILT" "$TONK_KERNEL_STORE/$NAME"
    touch "$TONK_KERNEL_STORE/$NAME"
  done
  ls -1t "$TONK_KERNEL_STORE" | tail -n +$((KEEP + 1)) | while IFS= read -r OLD; do
    rm -f "${TONK_KERNEL_STORE:?}/$OLD"
  done
  for KEPT in "$TONK_KERNEL_STORE"/*.wasm; do
    [ -f "$KEPT" ] || continue
    [ -f "$HOST_DIR/kernel/${KEPT##*/}" ] || cp "$KEPT" "$HOST_DIR/kernel/"
  done
fi

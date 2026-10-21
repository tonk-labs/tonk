#!/bin/sh
# Build what the spike's site serves: the embedder (the host's own, loaded
# by the bootloader with `importScripts`) and the kernel, under the BLAKE3
# hash a page pins it by. Then `node serve.mjs 8191` and open
# http://localhost:8191/?kind=tonk&kernel=<hash>, or drive it with
# `node drive.mjs <that url>`.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
cd "$root"
cargo build -p kernel-host --lib --release --target wasm32-unknown-unknown
wasm-bindgen --target no-modules --no-typescript --out-dir "$here" --out-name embedder \
    target/wasm32-unknown-unknown/release/kernel_host.wasm
cargo build -p tonk-kernel --release --target wasm32-wasip3
kernel=target/wasm32-wasip3/release/tonk_kernel.wasm
hash=$(cargo run -q -p kernel-host --release --example hash -- "$kernel" | cut -d' ' -f1)
mkdir -p "$here/_kernel"
cp "$kernel" "$here/_kernel/$hash.wasm"
echo "kernel $hash"

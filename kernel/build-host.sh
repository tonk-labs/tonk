#!/bin/sh
# Build what a deployment serves under `/_host/` for spaces that run a
# kernel: the embedder a space's bootloader loads (`embedder.js` and its
# wasm), and the kernel the deployment installs into a space it creates,
# under its content identifier, named in `kernel.json`. The app's build
# copies `dist/_host` into its own output when it is there.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cd "$here"
out="$here/dist/_host"
rm -rf "$out"
mkdir -p "$out/kernel"
cargo build -p kernel-host --lib --release --target wasm32-unknown-unknown
wasm-bindgen --target no-modules --no-typescript --out-dir "$out" --out-name embedder \
    target/wasm32-unknown-unknown/release/kernel_host.wasm
cargo build -p tonk-kernel --release --target wasm32-wasip3
kernel=target/wasm32-wasip3/release/tonk_kernel.wasm
id=$(cargo run -q -p kernel-host --release --example kernel-id -- "$kernel")
cp "$kernel" "$out/kernel/$id.wasm"
printf '{"kernel":"%s"}\n' "$id" > "$out/kernel.json"
echo "kernel $id"

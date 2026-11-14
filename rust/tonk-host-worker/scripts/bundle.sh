#!/usr/bin/env bash
# Bundle the space's kernel the Worker is deployed with into kernel/: the
# component with its core modules as skeletons, and every module the
# polyfill makes, compiled ahead of time (see README.md). Run from this
# directory before `worker-build`, in the repository's development shell.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
target="${CARGO_TARGET_DIR:-$root/target}"

# The embedder a recording run boots the kernel with: Node, which compiles
# at run time, records what a Worker cannot compile.
cargo build --manifest-path "$root/Cargo.toml" -p tonk-kernel-host \
  --features volatile --target wasm32-unknown-unknown --release
"${WASM_BINDGEN_BIN:-wasm-bindgen}" --target web --out-dir "$work/embedder" \
  "$target/wasm32-unknown-unknown/release/tonk_kernel_host.wasm"

bundle() {
  cargo run --quiet --release --manifest-path "$root/rust/tonk-kernel-bundle/Cargo.toml" \
    --features connection-invites -- "$@"
}
bundle component "$work/kernel.component"
node --experimental-wasm-jspi "$here/scripts/record.mjs" \
  "$work/embedder" "$work/kernel.component" "$work/recorded"
bundle bundle "$work/kernel.component" "$work/recorded" "$here/kernel"

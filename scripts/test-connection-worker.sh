#!/bin/sh
# Local production-Wasm revocation adapter/restart test. Never deploys.
set -eu
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
build_dir=$(mktemp -d "${TMPDIR:-/tmp}/tonk-connection-worker.XXXXXX")
trap 'rm -rf "$build_dir"' EXIT HUP INT TERM
# Install scripts/worker-test-tools with npm ci, then set NODE_PATH to its node_modules.
# Optional TONK_RELEASED_CLI_FIXTURES points to extracted official Linux CLI tarballs.
# Nix environments may instead supply the same modules through NODE_PATH.
if [ -z "${NODE_PATH:-}" ]; then
  wrangler_bin=$(command -v wrangler)
  wrangler_root=$(CDPATH= cd -- "$(dirname -- "$wrangler_bin")/.." && pwd)
  NODE_PATH="$wrangler_root/lib/node_modules"
  export NODE_PATH
fi
node -e 'require("miniflare")'
cd "$repo_root/rust/tonk-access-service"
worker-build --dev --no-opt --out-dir "$build_dir/build" -- --offline
cd "$repo_root"
TONK_CONNECTION_WORKER_SHIM="$build_dir/build/worker/shim.mjs" \
  cargo test --offline -p tonk-access-service --features helpers --test connections \
  connection_worker_standard_revocation_survives_persisted_kv_restart -- --ignored --nocapture

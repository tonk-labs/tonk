#!/bin/bash

# Build script for WASM bindings

set -e

echo "Building WASM bindings for tonk-core..."

# Ensure wasm-pack installed
if ! command -v wasm-pack &>/dev/null; then
  echo "wasm-pack not installed"
  exit 1
fi

# Clean previous builds
echo "Cleaning previous builds..."
rm -rf pkg pkg-node pkg-browser

# Build for web target
echo "Building for web target (wasm-browser)..."
RUSTFLAGS="--cfg getrandom_backend=\"wasm_js\"" \
  wasm-pack build --target web --out-dir pkg-browser \
  --features wasm-browser

# Build for Node.js
echo "Building for Node.js target..."
RUSTFLAGS="--cfg getrandom_backend=\"wasm_js\"" \
  wasm-pack build --target nodejs --out-dir pkg-node -- --features wasm-node

echo "WASM build completed successfully!"
echo ""
echo "Build outputs:"
echo "  - Web: ./pkg-browser/"
echo "  - Node.js: ./pkg-node/"
echo ""
echo "Next steps:"
echo "  - Run Node.js examples: cd examples/node && npm install && npm run example:basic"
echo "  - Run integration tests: cd examples/node && npm test"
echo "  - View browser example: open examples/browser/index.html"

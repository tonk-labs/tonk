#!/bin/bash

# Build script for WASM bindings

set -e

echo "Building WASM bindings for tonk-core..."

# Install wasm-pack if not already installed
if ! command -v wasm-pack &>/dev/null; then
  echo "wasm-pack not installed"
  exit 1
fi

# Clean previous builds
echo "Cleaning previous builds..."
rm -rf pkg pkg-node pkg-bundler

# Build for web target (ES modules)
echo "Building for web target..."
wasm-pack build --target web --out-dir pkg --no-typescript

# Generate TypeScript definitions
echo "Generating TypeScript definitions..."
wasm-pack build --target web --out-dir pkg --typescript-only

# Build for Node.js
echo "Building for Node.js target..."
wasm-pack build --target nodejs --out-dir pkg-node

# Build for bundlers (webpack, rollup, etc.)
echo "Building for bundler target..."
wasm-pack build --target bundler --out-dir pkg-bundler

echo "WASM build completed successfully!"
echo ""
echo "Build outputs:"
echo "  - Web (ES modules): ./pkg/"
echo "  - Node.js: ./pkg-node/"
echo "  - Bundlers: ./pkg-bundler/"

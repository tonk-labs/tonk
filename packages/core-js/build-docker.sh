#!/bin/bash

set -e

echo "🐳 Building @tonk/core using Docker-built WASM..."

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "🧹 Cleaning previous builds..."
rm -rf dist

mkdir -p dist

echo ""
echo "🔨 Building WASM bindings using Docker..."
cd ../core
./build-wasm-docker.sh

echo ""
echo "📦 Copying WASM files..."
cp -r pkg-browser/* ../core-js/dist/

cd ../core-js

echo ""
echo "🔨 Building TypeScript wrapper..."
npm run embed-wasm
npm run build:wrapper

echo ""
echo "✅ Build complete!"
echo ""
echo "Files in dist/:"
ls -la dist/
echo ""
echo "💡 This build uses WASM compiled for Amazon Linux 2023 EC2 compatibility"

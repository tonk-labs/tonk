#!/bin/bash

set -e

echo "🐳 Building WASM using Docker (Amazon Linux 2023 environment)..."
echo ""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

IMAGE_NAME="tonk-core-builder"
CONTAINER_NAME="tonk-core-build-$$"

echo "📦 Building Docker image..."
docker build -f Dockerfile.build -t "$IMAGE_NAME" .

echo ""
echo "🧹 Cleaning previous builds..."
rm -rf pkg pkg-node pkg-browser target

echo ""
echo "🔨 Building WASM for web target (wasm-browser)..."
echo "   Using .cargo/config.toml settings (matches EC2)..."
docker run --rm \
  --name "${CONTAINER_NAME}-browser" \
  -v "$SCRIPT_DIR:/build" \
  -w /build \
  "$IMAGE_NAME" \
  sh -c 'rm -rf target && wasm-pack build --target web --out-dir pkg-browser --features wasm-browser'

echo ""
echo "✅ Docker WASM build completed successfully!"
echo ""
echo "Build outputs:"
echo "  - Web: ./pkg-browser/"
echo ""
echo "📊 Build environment info:"
docker run --rm "$IMAGE_NAME" sh -c 'echo "Rust: $(rustc --version)" && echo "Target: $(rustc -vV | grep host | cut -d" " -f2)"'
echo ""
echo "💡 These WASM files are compatible with Amazon Linux 2023 EC2 instances"

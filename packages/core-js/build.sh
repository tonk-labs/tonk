#!/bin/bash

set -e

echo "Building @tonk/core..."

# Clean previous builds
echo "Cleaning previous builds..."
rm -rf dist

# Create dist directory
mkdir -p dist

# Build WASM in the core package
echo "Building WASM bindings..."
cd ../core
bun run build:browser

# Copy built files
echo "Copying WASM files..."
cp -r pkg-browser/* ../core-js/dist/

# Go back to package directory
cd ../core-js

# Build TypeScript wrapper
echo "Building TypeScript wrapper..."
bun run build:wasm
bun run build:wrapper

echo "Build complete!"
echo ""
echo "Files in dist/:"
ls -la dist/

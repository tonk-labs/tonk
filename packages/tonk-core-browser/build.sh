#!/bin/bash

set -e

echo "Building @tonk/core-browser-wasm package..."

# Clean previous builds
echo "Cleaning previous builds..."
rm -rf dist

# Create dist directory
mkdir -p dist

# Build WASM in the core package
echo "Building WASM bindings..."
cd ../core
npm run build:browser

# Copy built files
echo "Copying WASM files..."
cp -r pkg-browser/* ../tonk-core-browser-wasm/dist/

# Go back to package directory
cd ../tonk-core-browser-wasm

# Build TypeScript wrapper
echo "Building TypeScript wrapper..."
npm run build:wrapper

echo "Build complete!"
echo ""
echo "Files in dist/:"
ls -la dist/
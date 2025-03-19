#!/usr/bin / env node

/**
 * This script ensures the correct WASM binary is cached by the service worker.
 * It should be run after the webpack build process completes.
 */

const fs = require('fs');
const path = require('path');

// Configuration
const distDir = path.resolve(__dirname, '../dist');
const serviceWorkerPath = path.join(distDir, 'service-worker.js');

// Find the WASM file in the dist directory
function findWasmFile() {
  const files = fs.readdirSync(distDir);
  const wasmFile = files.find(file => file.endsWith('.wasm'));

  if (!wasmFile) {
    console.error('No WASM file found in the dist directory');
    process.exit(1);
  }

  return wasmFile;
}

// Update the service worker to cache the correct WASM file
function updateServiceWorker(wasmFile) {
  if (!fs.existsSync(serviceWorkerPath)) {
    console.error('Service worker file not found at:', serviceWorkerPath);
    process.exit(1);
  }

  let swContent = fs.readFileSync(serviceWorkerPath, 'utf8');

  // Check if the service worker already contains a WASM file reference
  const wasmPattern = /["']([^"']+\.wasm)["']/g;
  const wasmMatches = [...swContent.matchAll(wasmPattern)];

  if (wasmMatches.length === 0) {
    console.error('No WASM file reference found in service worker');
    process.exit(1);
  }

  // Replace all occurrences of WASM files with the current one
  for (const match of wasmMatches) {
    const oldWasmPath = match[1];
    // If the path starts with a slash, we need to add it to our replacement
    const prefix = oldWasmPath.startsWith('/') ? '/' : '';
    swContent = swContent.replace(oldWasmPath, `${prefix}${wasmFile}`);
  }

  // Write the updated service worker back to disk
  fs.writeFileSync(serviceWorkerPath, swContent);
  console.log(`Service worker updated to cache WASM file: ${wasmFile}`);
}

// Main function
function main() {
  try {
    console.log('Checking dist folder for WASM binary...');
    const wasmFile = findWasmFile();
    console.log(`Found WASM file: ${wasmFile}`);

    console.log('Updating service worker...');
    updateServiceWorker(wasmFile);

    console.log('Post-build WASM cache update completed successfully');
  } catch (error) {
    console.error('Error updating WASM cache:', error);
    process.exit(1);
  }
}

// Run the script
main(); 

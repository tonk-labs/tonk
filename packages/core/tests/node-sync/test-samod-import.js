#!/usr/bin/env node

// Test script to check what's available in the samod Node.js package

console.log('Testing samod Node.js import...');

try {
  // Try to import samod from the local package
  const samodPath = '../../../samod/samod/pkg-node/samod.js';
  console.log(`Attempting to import from: ${samodPath}`);

  const samod = require(samodPath);
  console.log('✅ Successfully imported samod');
  console.log('Available exports:', Object.keys(samod));

  // Check if there's a __wasm export (this is common for wasm-bindgen)
  if (samod.__wasm) {
    console.log('WASM exports:', Object.keys(samod.__wasm));
  }

  // Try to find Repo-like exports
  for (const [key, value] of Object.entries(samod)) {
    if (typeof value === 'function') {
      console.log(`Function: ${key}`);
    } else if (typeof value === 'object' && value !== null) {
      console.log(`Object: ${key}`, Object.keys(value));
    } else {
      console.log(`${typeof value}: ${key} = ${value}`);
    }
  }
} catch (error) {
  console.error('❌ Failed to import samod:', error.message);
  console.error('Full error:', error);
}

// Also try importing as ES module
try {
  console.log('\n--- Trying ES Module import ---');
  // Note: This needs to be dynamic import since we're in CommonJS context
  import(samodPath)
    .then(samodESM => {
      console.log('✅ Successfully imported samod as ES module');
      console.log('ES Module exports:', Object.keys(samodESM));
    })
    .catch(error => {
      console.error('❌ ES Module import failed:', error.message);
    });
} catch (error) {
  console.error('❌ ES Module import failed:', error.message);
}

#!/usr/bin/env bun

/**
 * Copies WASM from @tonk/core npm package to public/ for dev server.
 * Uses require.resolve() to find the package regardless of hoisting.
 */

import { cpSync, existsSync } from 'fs';
import { dirname, join } from 'path';
import { createRequire } from 'module';

const require = createRequire(import.meta.url);

function getTonkCoreWasmPath(): string {
  try {
    const tonkCorePkg = require.resolve('@tonk/core/package.json');
    const tonkCoreRoot = dirname(tonkCorePkg);
    return join(tonkCoreRoot, 'dist/tonk_core_bg.wasm');
  } catch {
    throw new Error('Cannot find @tonk/core package. Run: bun install');
  }
}

const wasmPath = getTonkCoreWasmPath();
const destPath = join(__dirname, '../public/tonk_core_bg.wasm');

if (!existsSync(wasmPath)) {
  console.error(`Error: WASM not found at ${wasmPath}`);
  console.error('Run: bun install');
  process.exit(1);
}

cpSync(wasmPath, destPath);
console.log(`[setup-wasm] Copied WASM to public/tonk_core_bg.wasm`);

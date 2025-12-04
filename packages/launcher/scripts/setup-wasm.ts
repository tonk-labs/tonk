#!/usr/bin/env bun

/**
 * Copies WASM from @tonk/core package to public/ for dev server.
 * Works with both npm packages and workspace-linked local packages.
 */

import { cpSync, existsSync } from 'fs';
import { dirname, join, resolve } from 'path';
import { createRequire } from 'module';

const require = createRequire(import.meta.url);
const destPath = join(__dirname, '../public/tonk_core_bg.wasm');

function getTonkCoreWasmPath(): string | null {
  // Try 1: Resolve from workspace or node_modules
  try {
    const tonkCorePkg = require.resolve('@tonk/core/package.json');
    const tonkCoreRoot = dirname(tonkCorePkg);
    const wasmPath = join(tonkCoreRoot, 'dist/tonk_core_bg.wasm');
    if (existsSync(wasmPath)) {
      return wasmPath;
    }
  } catch {
    // Package not installed yet
  }

  // Try 2: Direct workspace path (for local dev before install)
  const workspacePath = resolve(__dirname, '../../core-js/dist/tonk_core_bg.wasm');
  if (existsSync(workspacePath)) {
    return workspacePath;
  }

  return null;
}

const wasmPath = getTonkCoreWasmPath();

// If WASM already exists in public/, we're good
if (existsSync(destPath) && !wasmPath) {
  console.log(`[setup-wasm] WASM already exists at public/tonk_core_bg.wasm`);
  process.exit(0);
}

if (!wasmPath) {
  console.error(`Error: WASM not found. Either:`);
  console.error(`  1. Run: pnpm --filter @tonk/core run build`);
  console.error(`  2. Or: pnpm install (if using published @tonk/core)`);
  process.exit(1);
}

cpSync(wasmPath, destPath);
console.log(`[setup-wasm] Copied WASM to public/tonk_core_bg.wasm`);

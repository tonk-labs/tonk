import { execSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';

const ROOT = resolve(__dirname, '../../..');
const LAUNCHER_DIR = join(ROOT, 'packages/launcher');
const OUTPUT_DIR = join(LAUNCHER_DIR, 'public/app');
const DIST_SW = join(LAUNCHER_DIR, 'dist-sw');
const DIST_RUNTIME = join(LAUNCHER_DIR, 'dist-runtime');

// Use require.resolve to find @tonk/core regardless of where node_modules is hoisted
function getTonkCoreWasmPath(): string {
  const require = createRequire(import.meta.url);
  try {
    const tonkCorePkg = require.resolve('@tonk/core/package.json');
    const tonkCoreRoot = dirname(tonkCorePkg);
    return join(tonkCoreRoot, 'dist/tonk_core_bg.wasm');
  } catch {
    throw new Error('Cannot find @tonk/core package. Run: bun install');
  }
}

const WASM_PATH = getTonkCoreWasmPath();

console.log('Building runtime...');

try {
  const env = {
    ...process.env,
    NODE_ENV: 'production',
    TONK_SERVER_URL: process.env.TONK_SERVER_URL || 'http://localhost:8081',
    TONK_SERVE_LOCAL: 'false',
  };

  // 1. Build SW
  console.log('Running vite build -c vite.sw.config.ts...');
  execSync('bunx vite build -c vite.sw.config.ts', {
    cwd: LAUNCHER_DIR,
    stdio: 'inherit',
    env,
  });

  // 2. Build Runtime App
  console.log('Running vite build -c vite.runtime.config.ts...');
  execSync('bunx vite build -c vite.runtime.config.ts', {
    cwd: LAUNCHER_DIR,
    stdio: 'inherit',
    env,
  });

  // 3. Copy to public/app
  console.log(`Copying artifacts to ${OUTPUT_DIR}...`);

  if (existsSync(OUTPUT_DIR)) {
    rmSync(OUTPUT_DIR, { recursive: true, force: true });
  }
  mkdirSync(OUTPUT_DIR, { recursive: true });

  // Copy Runtime App
  cpSync(DIST_RUNTIME, OUTPUT_DIR, { recursive: true });

  // Copy SW
  if (existsSync(join(DIST_SW, 'service-worker-bundled.js'))) {
    cpSync(
      join(DIST_SW, 'service-worker-bundled.js'),
      join(OUTPUT_DIR, 'service-worker-bundled.js')
    );
  } else {
    console.error('Error: service-worker-bundled.js not found in dist-sw');
    process.exit(1);
  }

  // Copy WASM using resolved path (works regardless of hoisting)
  if (existsSync(WASM_PATH)) {
    console.log(`Copying WASM from: ${WASM_PATH}`);
    cpSync(WASM_PATH, join(OUTPUT_DIR, 'tonk_core_bg.wasm'));
  } else {
    console.error(`Error: WASM not found at ${WASM_PATH}`);
    console.error('Run: bun install to ensure @tonk/core is installed');
    process.exit(1);
  }

  // Cleanup temp dists
  rmSync(DIST_SW, { recursive: true, force: true });
  rmSync(DIST_RUNTIME, { recursive: true, force: true });

  console.log('Success! Runtime built and copied to public/app');
} catch (e) {
  console.error('Failed to build runtime:', e);
  process.exit(1);
}

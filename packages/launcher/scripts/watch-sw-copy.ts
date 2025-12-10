import { watch } from 'node:fs';
import { copyFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';

const LAUNCHER_DIR = join(import.meta.dir, '..');
const DIST_SW = join(LAUNCHER_DIR, 'dist-sw');
const OUTPUT_DIR = join(LAUNCHER_DIR, 'public/app');
const SW_FILE = 'service-worker-bundled.js';

console.log('[watch-sw-copy] Starting service worker copy watcher...');
console.log(`[watch-sw-copy] Watching: ${DIST_SW}`);
console.log(`[watch-sw-copy] Output: ${OUTPUT_DIR}/${SW_FILE}`);

// Ensure output directory exists
await mkdir(OUTPUT_DIR, { recursive: true });

// Initial delay to let vite build complete first
await new Promise(resolve => setTimeout(resolve, 2000));

// Copy function with retry
async function copyWithRetry(retries = 3) {
  for (let i = 0; i < retries; i++) {
    try {
      const src = join(DIST_SW, SW_FILE);
      const dest = join(OUTPUT_DIR, SW_FILE);
      await copyFile(src, dest);
      console.log(`[watch-sw-copy] Copied ${SW_FILE} to public/app`);
      return;
    } catch (err) {
      if (i < retries - 1) {
        await new Promise(resolve => setTimeout(resolve, 500));
      } else {
        console.error(
          `[watch-sw-copy] Failed to copy after ${retries} attempts:`,
          err
        );
      }
    }
  }
}

// Watch for changes in dist-sw
watch(DIST_SW, { recursive: false }, async (_eventType, filename) => {
  if (filename === SW_FILE) {
    console.log(`[watch-sw-copy] Detected change in ${filename}, copying...`);
    // Small delay to ensure file is fully written
    await new Promise(resolve => setTimeout(resolve, 100));
    await copyWithRetry();
  }
});

// Do initial copy
await copyWithRetry();

// Keep the process running
console.log('[watch-sw-copy] Watching for changes...');

/**
 * Singleton WASM initialization to prevent race conditions.
 * All code paths that need WASM should call ensureWasmInitialized()
 * instead of calling initializeTonk() directly.
 */
import { initializeTonk } from '@tonk/core/slim';
import { logger } from './utils/logging';

let wasmInitPromise: Promise<void> | null = null;
let wasmInitialized = false;

/**
 * Ensures WASM is initialized exactly once.
 * Safe to call from multiple concurrent code paths.
 *
 * @returns Promise that resolves when WASM is ready
 */
export async function ensureWasmInitialized(): Promise<void> {
  // Already initialized - fast path
  if (wasmInitialized) {
    logger.debug('WASM already initialized');
    return;
  }

  // Initialization in progress - return existing promise
  if (wasmInitPromise) {
    logger.debug('WASM initialization in progress, waiting...');
    return wasmInitPromise;
  }

  // First call - start initialization
  logger.debug('Starting WASM initialization');

  wasmInitPromise = (async () => {
    try {
      const wasmUrl = `/tonk_core_bg.wasm`;
      // Cache-bust to ensure fresh WASM on updates
      const cacheBustUrl = `${wasmUrl}?t=${Date.now()}`;
      await initializeTonk({ wasmPath: cacheBustUrl });
      wasmInitialized = true;
      logger.info('WASM initialization completed');
    } catch (error) {
      // Reset state so retry is possible
      wasmInitPromise = null;
      wasmInitialized = false;
      logger.error('WASM initialization failed', {
        error: error instanceof Error ? error.message : String(error),
      });
      throw error;
    }
  })();

  return wasmInitPromise;
}

/**
 * Check if WASM is currently initialized.
 * Useful for logging and diagnostics.
 */
export function isWasmInitialized(): boolean {
  return wasmInitialized;
}

/**
 * Reset WASM initialization state.
 * Only for testing - do not use in production code.
 */
export function resetWasmInitialization(): void {
  wasmInitPromise = null;
  wasmInitialized = false;
}

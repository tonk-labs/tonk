import init, { initSync } from './tonk_core.js';
import type { TonkConfig } from './core.js';
import { WASM_BASE64 } from './generated/wasm-data.js';

// Track initialization state
let initialized = false;
let initializationPromise: Promise<void> | null = null;

/**
 * Initialize the Tonk WASM module.
 *
 * This function must be called before using any other Tonk functionality.
 * It's safe to call multiple times - subsequent calls will be no-ops.
 *
 * @param config - Optional configuration for initialization
 *
 * @example
 * ```typescript
 * // Basic initialization
 * await initializeTonk();
 *
 * // With custom WASM path
 * await initializeTonk({
 *   wasmPath: '/assets/tonk_core_bg.wasm'
 * });
 * ```
 */
export async function initializeTonk(config?: TonkConfig): Promise<void> {
  // If already initialized, return immediately
  if (initialized) return;

  // If initialization is in progress, return the existing promise
  if (initializationPromise) return initializationPromise;

  // Start initialization
  initializationPromise = (async () => {
    try {
      if (config?.wasmPath) {
        await init(config.wasmPath);
      } else {
        await init();
      }
      initialized = true;
    } catch (error) {
      // Reset on failure so it can be retried
      initializationPromise = null;
      throw error;
    }
  })();

  return initializationPromise;
}

/**
 * Initialize the Tonk WASM module with embedded WASM data.
 * This is used by the auto-initializing main entrypoint.
 *
 * @param config - Optional configuration for initialization
 */
export async function initializeTonkWithEmbeddedWasm(
  config?: TonkConfig
): Promise<void> {
  // If already initialized, return immediately
  if (initialized) return;

  // If initialization is in progress, return the existing promise
  if (initializationPromise) return initializationPromise;

  // Start initialization
  initializationPromise = (async () => {
    try {
      if (config?.wasmPath) {
        await init(config.wasmPath);
      } else {
        const wasmBlob = Uint8Array.from(atob(WASM_BASE64), c =>
          c.charCodeAt(0)
        );
        initSync(wasmBlob);
      }
      initialized = true;
    } catch (error) {
      // Reset on failure so it can be retried
      initializationPromise = null;
      throw error;
    }
  })();

  return initializationPromise;
}

/**
 * Check if the WASM module has been initialized.
 *
 * @returns true if initialized, false otherwise
 */
export function isInitialized(): boolean {
  return initialized;
}

/**
 * Ensure the WASM module is initialized before proceeding.
 * Throws an error with a helpful message if not initialized.
 *
 * @internal
 */
export function ensureInitialized(): void {
  if (!initialized) {
    throw new Error(
      'Tonk WASM module not initialized. Please call initializeTonk() before using any Tonk functionality.\n\n' +
        'Example:\n' +
        '  import { initializeTonk, createSyncEngine } from "@tonk/core-browser-wasm/slim";\n' +
        '  \n' +
        '  await initializeTonk({ wasmPath: "/path/to/tonk_core_bg.wasm" });\n' +
        '  const engine = await createSyncEngine();'
    );
  }
}


import type { TonkConfig } from './core.js';
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
export declare function initializeTonk(config?: TonkConfig): Promise<void>;
/**
 * Initialize the Tonk WASM module with embedded WASM data.
 * This is used by the auto-initializing main entrypoint.
 *
 * @param config - Optional configuration for initialization
 */
export declare function initializeTonkWithEmbeddedWasm(config?: TonkConfig): Promise<void>;
/**
 * Check if the WASM module has been initialized.
 *
 * @returns true if initialized, false otherwise
 */
export declare function isInitialized(): boolean;
/**
 * Ensure the WASM module is initialized before proceeding.
 * Throws an error with a helpful message if not initialized.
 *
 * @internal
 */
export declare function ensureInitialized(): void;
//# sourceMappingURL=init.d.ts.map
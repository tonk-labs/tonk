import { initializeTonkWithEmbeddedWasm, isInitialized } from './init.js';
import type { TonkConfig } from './core.js';
import { TonkCore, Bundle, type NodeMetadata, type DirectoryEntry, type BundleEntry, TonkError, ConnectionError, FileSystemError, BundleError } from './core.js';
/**
 * Initialize the Tonk WASM module.
 *
 * This function is provided for compatibility, but the module is already
 * auto-initialized when importing from the main entrypoint.
 *
 * @param config - Optional configuration for initialization
 *
 * @example
 * ```typescript
 * // This is optional since auto-initialization already happened
 * await initializeTonk();
 *
 * // With custom WASM path (will re-initialize)
 * await initializeTonk({
 *   wasmPath: '/assets/tonk_core_bg.wasm'
 * });
 * ```
 */
export declare const initializeTonk: typeof initializeTonkWithEmbeddedWasm;
/**
 * Check if the WASM module has been initialized.
 *
 * @returns true if initialized, false otherwise
 */
export { isInitialized };
export { TonkCore, Bundle, type NodeMetadata, type DirectoryEntry, type BundleEntry, type TonkConfig, TonkError, ConnectionError, FileSystemError, BundleError, };
/**
 * Create a new Tonk Core
 *
 * @returns A new TonkCore instance
 * @throws {Error} If Tonk creation fails
 *
 * @example
 * ```typescript
 * const tonk = await createTonk();
 * const peerId = await tonk.getPeerId();
 * console.log('Created Tonk with peer ID:', peerId);
 * ```
 */
export declare const createTonk: () => Promise<TonkCore>;
/**
 * Create a new Tonk Core with a specific peer ID.
 *
 * @param peerId - The peer ID to use
 * @returns A new TonkCore instance
 * @throws {Error} If Tonk creation fails or peer ID is invalid
 *
 * @example
 * ```typescript
 * const tonk = await createTonkWithPeerId('my-custom-peer-id');
 * ```
 */
export declare const createTonkWithPeerId: (peerId: string) => Promise<TonkCore>;
/**
 * Create a Tonk Core from an existing bundle
 * @param bundle - The Bundle instance from which to load
 * @returns A new TonkCore instance
 * @throws {Error} If Tonk creation fails or bundle is invalid
 *
 * @example
 * ```typescript
 * const tonk = await createTonkFromBundle(bundle);
 * ```
 */
export declare const createTonkFromBundle: (bundle: Bundle) => Promise<TonkCore>;
/**
 * Create a Tonk Core from bundle data
 * @param bundle - The Bundle data from which to load
 * @returns A new TonkCore instance
 * @throws {Error} If Tonk creation fails or bundle is invalid
 *
 * @example
 * ```typescript
 * const data = bundle.toBytes();
 * const tonk = await createTonkFromBytes(data);
 * ```
 */
export declare const createTonkFromBytes: (data: Uint8Array) => Promise<TonkCore>;
/**
 * Create a bundle from existing data.
 *
 * @param data - Binary data representing a serialized bundle
 * @returns A new Bundle instance
 * @throws {BundleError} If the data is invalid or corrupted
 *
 * @example
 * ```typescript
 * const bundle = await createBundleFromBytes(existingBundleData);
 * const keys = await bundle.listKeys();
 * ```
 */
export declare const createBundleFromBytes: (data: Uint8Array) => Promise<Bundle>;
export declare const VERSION = "0.1.0";
//# sourceMappingURL=index.d.ts.map
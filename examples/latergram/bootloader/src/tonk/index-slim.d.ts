/**
 * Slim entrypoint for @tonk/core-browser-wasm
 *
 * This entrypoint does NOT auto-initialize the WASM module.
 * You must explicitly call initializeTonk() before using any functionality.
 *
 * @example
 * ```typescript
 * import { initializeTonk, createTonk } from '@tonk/core-browser-wasm/slim';
 *
 * // Initialize with explicit WASM path
 * await initializeTonk({
 *   wasmPath: new URL('@tonk/core-browser-wasm/wasm', import.meta.url).href
 * });
 *
 * // Now you can use Tonk
 * const tonk = await createTonk();
 * ```
 */
export { initializeTonk, isInitialized } from './init.js';
export { TonkCore, Bundle, type NodeMetadata, type DirectoryEntry, type BundleEntry, type TonkConfig, TonkError, ConnectionError, FileSystemError, BundleError, createFactoryFunctions, } from './core.js';
/**
 * Create a new Tonk Core with an auto-generated peer ID.
 *
 * @returns A new TonkCore instance
 * @throws {Error} If Tonk creation fails or WASM not initialized
 *
 * @example
 * ```typescript
 * await initializeTonk();
 * const tonk = await createTonk();
 * const peerId = await tonk.getPeerId();
 * console.log('Created Tonk with peer ID:', peerId);
 * ```
 */
export declare const createTonk: () => Promise<import("./core.js").TonkCore>;
/**
 * Create a new Tonk Core with a specific peer ID.
 *
 * @param peerId - The peer ID to use
 * @returns A new TonkCore instance
 * @throws {Error} If Tonk creation fails, peer ID is invalid, or WASM not initialized
 *
 * @example
 * ```typescript
 * await initializeTonk();
 * const tonk = await createTonkWithPeerId('my-custom-peer-id');
 * ```
 */
export declare const createTonkWithPeerId: (peerId: string) => Promise<import("./core.js").TonkCore>;
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
export declare const createTonkFromBundle: (bundle: import("./core.js").Bundle) => Promise<import("./core.js").TonkCore>;
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
export declare const createTonkFromBytes: (data: Uint8Array) => Promise<import("./core.js").TonkCore>;
/**
 * Create a bundle from existing data.
 *
 * @param data - Binary data representing a serialized bundle
 * @returns A new Bundle instance
 * @throws {BundleError} If the data is invalid, corrupted, or WASM not initialized
 *
 * @example
 * ```typescript
 * await initializeTonk();
 * const bundle = await createBundleFromBytes(existingBundleData);
 * const keys = await bundle.listKeys();
 * ```
 */
export declare const createBundleFromBytes: (data: Uint8Array) => Promise<import("./core.js").Bundle>;
export declare const VERSION = "0.1.0";
//# sourceMappingURL=index-slim.d.ts.map
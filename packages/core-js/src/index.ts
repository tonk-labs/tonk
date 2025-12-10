import type { TonkConfig } from './core.js';
import {
  Bundle,
  type BundleEntry,
  BundleError,
  ConnectionError,
  // Factory function helper
  createFactoryFunctions,
  type DirectoryNode,
  // Types
  type DocumentData,
  type DocumentTimestamps,
  type DocumentWatcher,
  FileSystemError,
  type JsonValue,
  type Manifest,
  type RefNode,
  // Main classes
  TonkCore,
  // Error classes
  TonkError,
} from './core.js';
import { WASM_BASE64 } from './generated/wasm-data.js';
import { initializeTonkWithEmbeddedWasm, isInitialized } from './init.js';

// Auto-initialize on import for convenience
await initializeTonkWithEmbeddedWasm();

// Import the WASM module directly since we've auto-initialized
const wasmModule = await import('./tonk_core.js');

// Create factory functions with direct WASM module access
const factories = createFactoryFunctions(wasmModule);

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
export const initializeTonk = initializeTonkWithEmbeddedWasm;

/**
 * Check if the WASM module has been initialized.
 *
 * @returns true if initialized, false otherwise
 */
export { isInitialized };

// Re-export all wrapper classes and types
export {
  // Main classes
  TonkCore,
  Bundle,
  // Types
  type DocumentData,
  type DocumentWatcher,
  type Manifest,
  type BundleEntry,
  type TonkConfig,
  type DocumentTimestamps,
  type RefNode,
  type DirectoryNode,
  type JsonValue,
  // Error classes
  TonkError,
  ConnectionError,
  FileSystemError,
  BundleError,
};

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
export const createTonk = factories.createTonk;

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
export const createTonkWithPeerId = factories.createTonkWithPeerId;

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
export const createTonkFromBundle = factories.createTonkFromBundle;

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
export const createTonkFromBytes = factories.createTonkFromBytes;

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
export const createBundleFromBytes = factories.createBundleFromBytes;

// Version export
export const VERSION = '0.1.0';

// Export embedded WASM data for direct access
export { WASM_BASE64 };

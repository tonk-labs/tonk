/**
 * Slim entrypoint for @tonk/core-browser-wasm
 *
 * This entrypoint does NOT auto-initialize the WASM module.
 * You must explicitly call initializeTonk() before using any functionality.
 *
 * @example
 * ```typescript
 * import { initializeTonk, createSyncEngine } from '@tonk/core-browser-wasm/slim';
 *
 * // Initialize with explicit WASM path
 * await initializeTonk({
 *   wasmPath: new URL('@tonk/core-browser-wasm/wasm', import.meta.url).href
 * });
 *
 * // Now you can use Tonk
 * const engine = await createSyncEngine();
 * ```
 */

import { createFactoryFunctions } from './core.js';

// Export initialization functions
export { initializeTonk, isInitialized } from './init.js';

// Re-export wrapper classes and types
export {
  // Main classes
  SyncEngine,
  VirtualFileSystem,
  Repository,
  Bundle,

  // Types
  type NodeMetadata,
  type DirectoryEntry,
  type BundleEntry,
  type TonkConfig,

  // Error classes
  TonkError,
  ConnectionError,
  FileSystemError,
  BundleError,

  // Factory function helper
  createFactoryFunctions,
} from './core.js';

// Create factory functions with lazy-loaded WASM module
const lazyFactories = createFactoryFunctions();

/**
 * Create a new sync engine with an auto-generated peer ID.
 *
 * @returns A new SyncEngine instance
 * @throws {Error} If engine creation fails or WASM not initialized
 *
 * @example
 * ```typescript
 * await initializeTonk();
 * const engine = await createSyncEngine();
 * const peerId = await engine.getPeerId();
 * console.log('Created engine with peer ID:', peerId);
 * ```
 */
export const createSyncEngine = lazyFactories.createSyncEngine;

/**
 * Create a new sync engine with a specific peer ID.
 *
 * @param peerId - The peer ID to use
 * @returns A new SyncEngine instance
 * @throws {Error} If engine creation fails, peer ID is invalid, or WASM not initialized
 *
 * @example
 * ```typescript
 * await initializeTonk();
 * const engine = await createSyncEngineWithPeerId('my-custom-peer-id');
 * ```
 */
export const createSyncEngineWithPeerId =
  lazyFactories.createSyncEngineWithPeerId;

/**
 * Create a new empty bundle.
 *
 * @returns A new Bundle instance
 * @throws {BundleError} If bundle creation fails or WASM not initialized
 *
 * @example
 * ```typescript
 * await initializeTonk();
 * const bundle = await createBundle();
 * await bundle.put('key', new Uint8Array([1, 2, 3]));
 * ```
 */
export const createBundle = lazyFactories.createBundle;

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
export const createBundleFromBytes = lazyFactories.createBundleFromBytes;

// Version export
export const VERSION = '0.1.0';

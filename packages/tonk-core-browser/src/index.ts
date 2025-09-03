import { initializeTonkWithEmbeddedWasm, isInitialized } from './init.js';
import type { TonkConfig } from './core.js';
import {
  // Main classes
  SyncEngine,
  VirtualFileSystem,
  Repository,
  Bundle,

  // Types
  type NodeMetadata,
  type DirectoryEntry,
  type BundleEntry,

  // Error classes
  TonkError,
  ConnectionError,
  FileSystemError,
  BundleError,

  // Factory function helper
  createFactoryFunctions,
} from './core.js';

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
};

/**
 * Create a new sync engine with an auto-generated peer ID.
 *
 * @returns A new SyncEngine instance
 * @throws {Error} If engine creation fails
 *
 * @example
 * ```typescript
 * const engine = await createSyncEngine();
 * const peerId = await engine.getPeerId();
 * console.log('Created engine with peer ID:', peerId);
 * ```
 */
export const createSyncEngine = factories.createSyncEngine;

/**
 * Create a new sync engine with a specific peer ID.
 *
 * @param peerId - The peer ID to use
 * @returns A new SyncEngine instance
 * @throws {Error} If engine creation fails or peer ID is invalid
 *
 * @example
 * ```typescript
 * const engine = await createSyncEngineWithPeerId('my-custom-peer-id');
 * ```
 */
export const createSyncEngineWithPeerId = factories.createSyncEngineWithPeerId;

/**
 * Create a new empty bundle.
 *
 * @returns A new Bundle instance
 * @throws {BundleError} If bundle creation fails
 *
 * @example
 * ```typescript
 * const bundle = await createBundle();
 * await bundle.put('key', new Uint8Array([1, 2, 3]));
 * ```
 */
export const createBundle = factories.createBundle;

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

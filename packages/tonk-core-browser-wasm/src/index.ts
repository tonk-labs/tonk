import init, { initThreadPool } from '../dist/tonk_core.js';
import type { TonkConfig } from './wrappers.js';

// Track initialization state
let initialized = false;

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
 *
 * // With multi-threading
 * await initializeTonk({
 *   numThreads: 4
 * });
 * ```
 */
export async function initializeTonk(config?: TonkConfig): Promise<void> {
  if (initialized) return;

  if (config?.wasmPath) {
    await init(config.wasmPath);
  } else {
    await init();
  }

  // Initialize thread pool if threading is requested
  if (config?.numThreads && config.numThreads > 0) {
    await initThreadPool(config.numThreads);
  }

  initialized = true;
}

/**
 * Check if the WASM module has been initialized.
 *
 * @returns true if initialized, false otherwise
 */
export function isInitialized(): boolean {
  return initialized;
}

// Auto-initialize on import for convenience
await initializeTonk();

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
} from './wrappers.js';

// Re-export convenience factory functions
import { SyncEngine, Bundle } from './wrappers.js';

/**
 * Create a new sync engine with an auto-generated peer ID.
 *
 * @returns A new SyncEngine instance
 * @throws {Error} If engine creation fails
 *
 * @example
 * ```typescript
 * const engine = await createSyncEngine();
 * const peerId = await engine.peerId;
 * console.log('Created engine with peer ID:', peerId);
 * ```
 */
export const createSyncEngine = SyncEngine.create;

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
export const createSyncEngineWithPeerId = SyncEngine.createWithPeerId;

/**
 * Create a new empty bundle.
 *
 * @returns A new Bundle instance
 * @throws {BundleError} If bundle creation fails
 *
 * @example
 * ```typescript
 * const bundle = createBundle();
 * await bundle.put('key', new Uint8Array([1, 2, 3]));
 * ```
 */
export const createBundle = Bundle.create;

/**
 * Create a bundle from existing data.
 *
 * @param data - Binary data representing a serialized bundle
 * @returns A new Bundle instance
 * @throws {BundleError} If the data is invalid or corrupted
 *
 * @example
 * ```typescript
 * const bundle = createBundleFromBytes(existingBundleData);
 * const keys = await bundle.listKeys();
 * ```
 */
export const createBundleFromBytes = Bundle.fromBytes;

// Version export
export const VERSION = '0.1.0';

await initializeTonk();

// Re-export everything from the generated WASM bindings
export * from '../dist/tonk_core.js';
export { default as init } from '../dist/tonk_core.js';

// Re-export types
export type { InitInput, InitOutput } from '../dist/tonk_core.js';

// Convenience types for better TypeScript experience
export interface TonkConfig {
  wasmPath?: string;
  numThreads?: number;
}

// Helper function to initialize WASM with better error handling
export async function initializeTonk(config?: TonkConfig): Promise<void> {
  const init = (await import('../dist/tonk_core.js')).default;

  if (config?.wasmPath) {
    await init(config.wasmPath);
  } else {
    await init();
  }

  // Initialize thread pool if threading is requested
  if (config?.numThreads && config.numThreads > 0) {
    const { initThreadPool } = await import('../dist/tonk_core.js');
    await initThreadPool(config.numThreads);
  }
}

// Re-export classes with better documentation
export {
  WasmSyncEngine as SyncEngine,
  WasmVfs as VirtualFileSystem,
  WasmBundle as Bundle,
  WasmRepo as Repository,
} from '../dist/tonk_core.js';

// Export convenience functions
export {
  create_sync_engine as createSyncEngine,
  create_sync_engine_with_peer_id as createSyncEngineWithPeerId,
  create_bundle as createBundle,
  create_bundle_from_bytes as createBundleFromBytes,
} from '../dist/tonk_core.js';

import { setIsSlim } from './utils/wasmState.js';

// Export the engine
export * from './engine/index.js';

// Export the middleware
export {
  ls,
  mkDir,
  rm,
  readDoc,
  writeDoc,
  sync,
  listenToDoc,
} from './middleware/index.js';
export type { SyncOptions, DocumentId } from './middleware/index.js';

// This is useful for exploring the filesystem
export type { DocNode, DirNode, RefNode } from './documents/addressing.js';

// Export the core functionality
export * from './core/index.js';

// Re-export specific functions for easier access
export { configureSyncEngine, getSyncEngine } from './core/index.js';

// Export network adapters
export { IrohNetworkAdapter } from './adapters/index.js';

// Export WASM initialization for slim entry point users
export { initializeWasm } from '@automerge/automerge-repo/slim';
setIsSlim();

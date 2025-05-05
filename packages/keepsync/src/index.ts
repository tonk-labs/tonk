// Export the engine
export * from './engine/index.js';

// Export the middleware
export {readDoc, writeDoc, sync, listenToDoc} from './middleware/index.js';
export type {SyncOptions, DocumentId} from './middleware/index.js';

// Export the core functionality
export * from './core/index.js';

// Re-export specific functions for easier access
export {configureSyncEngine, getSyncEngine} from './core/syncConfig.js';

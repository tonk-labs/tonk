// Export the engine
export * from './engine/index.js';

// Export the middleware
export * from './middleware/index.js';

// Export the core functionality
export * from './core/index.js';

// Export the file system functionality
// export * from './fs/index.js';

// Re-export specific functions for easier access
export {
  configureSyncEngine,
  getSyncEngine,
  closeSyncEngine,
} from './core/syncConfig.js';

// Re-export file system functions for easier access
// export {
//   configureSyncedFileSystem,
//   getSyncedFileManager,
//   addFile,
//   removeFile,
//   getFile,
//   getAllFiles,
//   closeSyncedFileSystem,
// } from './core/syncedFiles.js';

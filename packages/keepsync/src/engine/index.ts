import {SyncEngine as SyncEngineClass} from './syncEngine.js';
import {SyncEngineOptions} from './types.js';

// Export the SyncEngine class
export {SyncEngine} from './syncEngine.js';

// Export types
export * from './types.js';

// Export individual modules for advanced usage
export * as connection from './connection/index.js';
// export * as messaging from './messaging/index.js';
export * as storage from './storage.js';

export const getSyncInstance = () => SyncEngineClass.getInstance();
export const configureSyncInstance = (options: SyncEngineOptions) =>
  SyncEngineClass.configureInstance(options);
export const resetSyncInstance = () => SyncEngineClass.resetInstance();

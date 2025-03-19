import {SyncEngine as SyncEngineClass} from './syncEngine';
import {SyncEngineOptions} from './types';

// Export the SyncEngine class
export {SyncEngine} from './syncEngine';

// Export types
export * from './types';

// Export individual modules for advanced usage
export * as connection from './connection';
export * as document from './document';
export * as messaging from './messaging';
export * as storage from './storage';

export const getSyncInstance = () => SyncEngineClass.getInstance();
export const configureSyncInstance = (options: SyncEngineOptions) =>
  SyncEngineClass.configureInstance(options);
export const resetSyncInstance = () => SyncEngineClass.resetInstance();

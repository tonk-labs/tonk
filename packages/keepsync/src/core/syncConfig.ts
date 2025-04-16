import {SyncEngine, SyncEngineOptions} from '../engine';
import {Repo} from '@automerge/automerge-repo';
import {logger} from '../utils/logger';

// Singleton instance of the SyncEngine
let syncEngineInstance: SyncEngine | null = null;

/**
 * Configure the global sync engine with the provided options
 * This should be called once at the application startup
 * @param options Configuration options for the SyncEngine
 * @returns The configured SyncEngine instance
 */
export function configureSyncEngine(options: SyncEngineOptions): SyncEngine {
  if (!syncEngineInstance) {
    logger.info('Creating new SyncEngine instance');
    syncEngineInstance = new SyncEngine(options);
  } else {
    logger.warn('SyncEngine instance already exists. Ignoring new options.');
  }

  return syncEngineInstance;
}

/**
 * Get the sync engine instance
 * @returns The SyncEngine instance or null if not created yet
 */
export function getSyncEngine(): SyncEngine | null {
  if (!syncEngineInstance) {
    logger.warn('Sync engine not created yet');
    return null;
  }

  return syncEngineInstance;
}

/**
 * Get the Automerge Repo instance from the SyncEngine
 * @returns The Repo instance or null if SyncEngine not created yet
 */
export function getRepo(): Repo | null {
  const syncEngine = getSyncEngine();
  return syncEngine ? syncEngine.getRepo() : null;
}

/**
 * Reset the sync engine instance (mainly for testing purposes)
 */
export function resetSyncEngine(): void {
  syncEngineInstance = null;
}

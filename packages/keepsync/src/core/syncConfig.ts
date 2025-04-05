import {
  configureSyncInstance,
  getSyncInstance,
  SyncEngine,
} from '../engine/index.js';
import {SyncEngineOptions} from '../engine/types.js';
import {logger} from '../utils/logger.js';

/**
 * Configure the global sync engine with the provided options
 * This should be called once at the application startup
 */
export function configureSyncEngine(options: SyncEngineOptions): void {
  const syncEngine = getSyncInstance();

  if (!syncEngine) configureSyncInstance(options);
}

/**
 * Get the sync engine instance
 * If not initialized, it will initialize automatically with default settings
 */
export async function getSyncEngine(): Promise<SyncEngine | null> {
  const syncEngine = getSyncInstance();

  if (!syncEngine) {
    logger.warn('Sync engine not created yet');
    return null;
  }

  if (!syncEngine.isInitialized()) await syncEngine.init();

  return syncEngine;
}

/**
 * Close the sync engine connection
 * Should be called when the application is shutting down
 */
export function closeSyncEngine(): void {
  const syncEngine = getSyncInstance();
  if (syncEngine) syncEngine.close();
}

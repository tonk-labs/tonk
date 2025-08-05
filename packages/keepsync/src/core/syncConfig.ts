import { SyncEngine } from '../engine/index.js';
import { Repo, DocumentId } from '@automerge/automerge-repo/slim';
import { logger } from '../utils/logger.js';
import { SyncEngineOptions } from '../engine/syncEngine.js';
import { waitForWasm } from '../utils/wasmState.js';

// Singleton instance of the SyncEngine
let syncEngineInstance: SyncEngine | null = null;

/**
 * Configure the global sync engine with the provided options
 * This should be called once at the application startup
 * @param options Configuration options for the SyncEngine
 * @returns The configured SyncEngine instance
 */
export async function configureSyncEngine(
  options: SyncEngineOptions
): Promise<SyncEngine> {
  // Ensure WASM is ready before creating SyncEngine
  // HACK: the way we initialize WASM for the main entrypoint needs serious attention
  await waitForWasm();

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
 * Get the Automerge Repo instance from the SyncEngine
 * @returns The Repo instance or null if SyncEngine not created yet
 */
export function getRootId(): DocumentId | undefined {
  const syncEngine = getSyncEngine();
  return syncEngine ? syncEngine.getRootId() : undefined;
}

/**
 * Reset the sync engine instance (mainly for testing purposes)
 */
export function resetSyncEngine(): void {
  syncEngineInstance = null;
}

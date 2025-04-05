import {SyncedFileManager} from '../fs/syncedFileManager.js';
import {FileMetadata} from '../fs/types.js';
import {logger} from '../utils/logger.js';
import {getSyncEngine} from './syncConfig.js';

// Singleton instance of the synced file manager
let fileManager: SyncedFileManager | null = null;
let initPromise: Promise<SyncedFileManager | null> | null = null;

/**
 * Options for configuring the synced file system
 */
export interface SyncedFileSystemOptions {
  /**
   * Document ID for storing file metadata
   * @default 'files'
   */
  docId?: string;

  /**
   * Database name for storing file blobs
   * @default 'sync-engine-store'
   */
  dbName?: string;

  /**
   * Store name within the database for file blobs
   * @default 'file-blobs'
   */
  storeName?: string;
}

/**
 * Configure the global synced file system with the provided options
 * This should be called once at the application startup
 */
export function configureSyncedFileSystem(
  options: SyncedFileSystemOptions = {},
): void {
  if (!fileManager) {
    // We'll initialize it later when getSyncedFileManager is called
    // Just store the configuration for now
    initPromise = initSyncedFileSystem(options);
  }
}

/**
 * Initialize the synced file system
 * This is called automatically when getSyncedFileManager is called
 */
async function initSyncedFileSystem(
  options: SyncedFileSystemOptions = {},
): Promise<SyncedFileManager | null> {
  // Get the sync engine first
  const syncEngine = await getSyncEngine();
  if (!syncEngine) {
    logger.warn('Sync engine not created yet');
    return null;
  }

  // Create the file manager with the provided options
  fileManager = new SyncedFileManager(syncEngine, options.docId || 'files', {
    dbName: options.dbName || 'sync-engine-store',
    storeName: options.storeName || 'file-blobs',
  });

  // Initialize the file manager
  await fileManager.init();

  return fileManager;
}

/**
 * Get the global synced file manager instance
 * If not initialized, it will initialize automatically with default options
 */
export async function getSyncedFileManager(): Promise<SyncedFileManager | null> {
  if (!initPromise) {
    initPromise = initSyncedFileSystem();
  }

  return initPromise;
}

/**
 * Add a file to the synced file system
 * @param file The file to add
 * @returns Metadata for the added file
 */
export async function addFile(file: File): Promise<FileMetadata | null> {
  const manager = await getSyncedFileManager();
  if (!manager) return null;
  return manager.addFile(file);
}

/**
 * Remove a file from the synced file system
 * @param hash The hash of the file to remove
 */
export async function removeFile(hash: string): Promise<void> {
  const manager = await getSyncedFileManager();
  if (!manager) return;
  return manager.removeFile(hash);
}

/**
 * Get a file blob by its hash
 * @param hash The hash of the file to get
 * @returns The file blob or null if not found
 */
export async function getFile(hash: string): Promise<Blob | null> {
  const manager = await getSyncedFileManager();
  if (!manager) return null;
  return manager.getBlob(hash);
}

/**
 * Get all files in the synced file system
 * @returns Array of file metadata
 */
export async function getAllFiles(): Promise<FileMetadata[] | null> {
  const manager = await getSyncedFileManager();
  if (!manager) return null;
  return manager.getAllFiles();
}

/**
 * Close the synced file system
 * Should be called when the application is shutting down
 */
export function closeSyncedFileSystem(): void {
  fileManager = null;
  initPromise = null;
}

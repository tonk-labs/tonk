import {SyncEngine} from '../engine/index.js';
import {AutomergeFileManager} from './fileDoc.js';
import {FileMetadata} from './types.js';
import {logger} from '../utils/logger.js';

/**
 * A file manager that integrates with the SyncEngine for local-first file storage
 */
export class SyncedFileManager extends AutomergeFileManager {
  private syncEngine: SyncEngine;
  private docId: string;
  private initialized = false;
  private syncCallback: ((docId: string) => void) | null = null;
  private blobRequestHandlers: Map<string, (hash: string) => Promise<void>> =
    new Map();

  /**
   * Create a new SyncedFileManager
   * @param syncEngine The SyncEngine instance to use
   * @param docId The document ID to use for file metadata
   * @param options Options for the file manager
   */
  constructor(
    syncEngine: SyncEngine,
    docId: string = 'files',
    options: {
      dbName?: string;
      storeName?: string;
    } = {},
  ) {
    // Use the same database as the SyncEngine by default
    super({
      dbName: options.dbName || 'sync-engine-store',
      storeName: options.storeName || 'file-blobs',
    });

    this.syncEngine = syncEngine;
    this.docId = docId;
  }

  /**
   * Initialize the file manager and ensure the file document exists
   */
  async init(): Promise<void> {
    if (this.initialized) return;

    // Initialize the base file manager
    await super.init();

    // Check if the file document exists, create it if not
    const doc = await this.syncEngine.getDocument(this.docId);
    if (!doc) {
      logger.debugWithContext(
        'SyncedFileManager',
        `Creating new file document with ID: ${this.docId}`,
      );
      await this.syncEngine.createDocument(this.docId, {files: []});
    }

    // Register sync callback to detect when file metadata changes
    this.syncCallback = (syncedDocId: string) => {
      if (syncedDocId === this.docId) {
        this.handleDocumentSync().catch(err => {
          logger.error('Error handling document sync:', err);
        });
      }
    };

    // Register message handler for blob requests and responses
    this.syncEngine.registerMessageHandler('blob_request', async data => {
      await this.handleBlobRequest(data.hash);
    });

    this.syncEngine.registerMessageHandler('blob_response', async data => {
      await this.handleBlobResponse(data.hash, data.blob);
    });

    // Register the sync callback with the engine
    this.syncEngine.registerSyncCallback(this.syncCallback);

    this.initialized = true;

    // Check for missing blobs on initialization
    await this.checkForMissingBlobs();
  }

  /**
   * Handle document sync event by checking for missing blobs
   */
  private async handleDocumentSync(): Promise<void> {
    await this.checkForMissingBlobs();
  }

  /**
   * Check for missing blobs and request them from other clients
   */
  private async checkForMissingBlobs(): Promise<void> {
    const doc = await this.syncEngine.getDocument(this.docId);
    if (!doc || !doc.files) {
      return;
    }

    const missingHashes = await this.getMissingBlobs(doc);

    for (const hash of missingHashes) {
      await this.requestBlob(hash);
    }
  }

  /**
   * Request a blob from other clients
   */
  private async requestBlob(hash: string): Promise<void> {
    logger.debugWithContext(
      'SyncedFileManager',
      `Requesting blob with hash: ${hash}`,
    );

    // Create a promise that will be resolved when the blob is received
    const requestPromise = new Promise<void>((resolve, reject) => {
      // Set a timeout to reject the promise if the blob is not received
      const timeout = setTimeout(() => {
        this.blobRequestHandlers.delete(hash);
        reject(new Error(`Timeout waiting for blob: ${hash}`));
      }, 30000); // 30 second timeout

      // Store the handler that will be called when the blob is received
      this.blobRequestHandlers.set(hash, async () => {
        clearTimeout(timeout);
        this.blobRequestHandlers.delete(hash);
        resolve();
      });
    });

    // Send the request through the sync engine
    this.syncEngine.sendMessage({
      type: 'blob_request',
      hash,
    });

    try {
      await requestPromise;
      logger.debugWithContext(
        'SyncedFileManager',
        `Successfully received blob with hash: ${hash}`,
      );
    } catch (error) {
      logger.error(`Failed to receive blob with hash: ${hash}`, error);
      // Remove the handler if there was an error
      this.blobRequestHandlers.delete(hash);
    }
  }

  /**
   * Handle a blob request from another client
   */
  private async handleBlobRequest(hash: string): Promise<void> {
    // Check if we have the requested blob
    const hasBlob = await this.hasBlob(hash);
    if (!hasBlob) {
      return; // We don't have this blob, so we can't respond
    }

    // Get the blob and send it back
    const blob = await this.getBlob(hash);
    if (!blob) {
      return; // Shouldn't happen since we just checked, but just in case
    }

    // Convert blob to base64 for transmission
    const arrayBuffer = await blob.arrayBuffer();
    const base64 = this.arrayBufferToBase64(arrayBuffer);

    // Send the blob back through the sync engine
    this.syncEngine.sendMessage({
      type: 'blob_response',
      hash,
      blob: {
        data: base64,
        type: blob.type,
      },
    });
  }

  /**
   * Handle a blob response from another client
   */
  private async handleBlobResponse(
    hash: string,
    blobData: {data: string; type: string},
  ): Promise<void> {
    // Convert base64 back to blob
    const arrayBuffer = this.base64ToArrayBuffer(blobData.data);
    const blob = new Blob([arrayBuffer], {type: blobData.type});

    // Store the blob
    await this.storeBlob(hash, blob);

    // Resolve the request promise if there's a handler
    const handler = this.blobRequestHandlers.get(hash);
    if (handler) {
      await handler(hash);
    }
  }

  /**
   * Add a file to the system and update the synced document
   */
  async addFile(file: File): Promise<FileMetadata> {
    if (!this.initialized) {
      await this.init();
    }

    // First add the file to storage
    const metadata = await super.addFile(file);

    // Then update the document
    await this.syncEngine.updateDocument(this.docId, doc => {
      if (!doc.files) {
        doc.files = [];
      }

      // Check if file already exists
      const existingIndex = doc.files.findIndex(
        (f: FileMetadata) => f.hash === metadata.hash,
      );

      if (existingIndex >= 0) {
        // Update existing file
        doc.files[existingIndex] = metadata;
      } else {
        // Add new file
        doc.files.push(metadata);
      }
    });

    return metadata;
  }

  /**
   * Remove a file from the system and update the synced document
   */
  async removeFile(hash: string): Promise<void> {
    if (!this.initialized) {
      await this.init();
    }

    // First remove the file from storage
    await super.deleteBlob(hash);

    // Then update the document
    await this.syncEngine.updateDocument(this.docId, doc => {
      if (!doc.files) {
        return;
      }

      const index = doc.files.findIndex((f: FileMetadata) => f.hash === hash);
      if (index >= 0) {
        doc.files.splice(index, 1);
      }
    });
  }

  /**
   * Get all files from the synced document
   */
  async getAllFiles(): Promise<FileMetadata[]> {
    if (!this.initialized) {
      await this.init();
    }

    const doc = await this.syncEngine.getDocument(this.docId);
    if (!doc || !doc.files) {
      return [];
    }

    return doc.files;
  }

  /**
   * Sync missing blobs from another file manager
   * @param otherManager Another file manager instance to sync with
   */
  async syncMissingBlobs(
    otherManager: AutomergeFileManager,
  ): Promise<string[]> {
    if (!this.initialized) {
      await this.init();
    }

    const doc = await this.syncEngine.getDocument(this.docId);
    if (!doc || !doc.files) {
      return [];
    }

    const missingHashes = await this.getMissingBlobs(doc);
    const synced: string[] = [];

    for (const hash of missingHashes) {
      const blob = await otherManager.getBlob(hash);
      if (blob) {
        await this.storeBlob(hash, blob);
        synced.push(hash);
      }
    }

    return synced;
  }

  /**
   * Convert a base64 string to an ArrayBuffer
   * @param base64 The base64 string to convert
   * @returns An ArrayBuffer representation of the base64 string
   */
  private base64ToArrayBuffer(base64: string): ArrayBuffer {
    const binaryString = atob(base64);
    const bytes = new Uint8Array(binaryString.length);
    for (let i = 0; i < binaryString.length; i++) {
      bytes[i] = binaryString.charCodeAt(i);
    }
    return bytes.buffer;
  }

  /**
   * Convert an ArrayBuffer to a base64 string in a way that works for large files
   * @param buffer The ArrayBuffer to convert
   * @returns A base64 string representation of the buffer
   */
  private arrayBufferToBase64(buffer: ArrayBuffer): string {
    const bytes = new Uint8Array(buffer);
    let binary = '';
    const chunkSize = 1024; // Process in smaller chunks to avoid stack overflow

    for (let i = 0; i < bytes.length; i += chunkSize) {
      const chunk = bytes.slice(i, Math.min(i + chunkSize, bytes.length));
      binary += String.fromCharCode.apply(null, Array.from(chunk));
    }

    return btoa(binary);
  }

  /**
   * Clean up resources when the file manager is no longer needed
   */
  close(): void {
    if (this.syncCallback) {
      this.syncEngine.unregisterSyncCallback(this.syncCallback);
      this.syncCallback = null;
    }

    // Unregister message handlers
    this.syncEngine.unregisterMessageHandler('blob_request');
    this.syncEngine.unregisterMessageHandler('blob_response');

    // Clear any pending blob requests
    this.blobRequestHandlers.clear();

    // Close the underlying file manager
    super.close();

    this.initialized = false;
  }
}

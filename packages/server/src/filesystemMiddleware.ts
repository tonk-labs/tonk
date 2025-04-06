import {
  AutomergeFileSystemStorage,
  FileSystemStorageOptions,
} from './filesystemStorage.js';
import * as Automerge from '@automerge/automerge';
import {DocumentId} from './types.js';
import {logger} from './logger.js';
import {printFirstEntries} from './utils/printOutAutomerge.js';

export class FileSystemStorageMiddleware {
  private storage: AutomergeFileSystemStorage;
  private documents: Map<DocumentId, Automerge.Doc<any>> = new Map();
  private syncStates: Map<DocumentId, Automerge.SyncState> = new Map();
  private documentLocks: Map<DocumentId, Promise<void>> = new Map();
  private name: string;

  private fileWatchInterval: NodeJS.Timeout | null = null;

  constructor(
    options: FileSystemStorageOptions,
    name: string = 'FileSystemStorageMiddleware',
    private verbose: boolean = true,
    private fileWatchIntervalMs: number = 5000, // Check for external changes every 5 seconds by default
  ) {
    this.name = name;
    this.storage = new AutomergeFileSystemStorage(options, (color, message) =>
      this.log(color, message),
    );

    // Start watching for external file changes
    this.startFileWatcher();
  }

  private log(
    color: 'green' | 'red' | 'blue' | 'yellow',
    message: string,
  ): void {
    if (color === 'red') {
      logger.error(`[${this.name}] ${message}`);
    } else if (this.verbose) {
      logger.debugWithContext(this.name, message);
    }
  }

  public isInitialized(): boolean {
    return this.storage.isInitialized();
  }

  // Start watching for external file changes
  private startFileWatcher(): void {
    if (this.fileWatchInterval) {
      clearInterval(this.fileWatchInterval);
    }

    this.fileWatchInterval = setInterval(() => {
      this.checkForExternalChanges();
    }, this.fileWatchIntervalMs);

    this.log(
      'blue',
      `Started file watcher with interval of ${this.fileWatchIntervalMs}ms`,
    );
  }

  // Stop watching for external file changes
  private stopFileWatcher(): void {
    if (this.fileWatchInterval) {
      clearInterval(this.fileWatchInterval);
      this.fileWatchInterval = null;
      this.log('blue', 'Stopped file watcher');
    }
  }

  // Check all loaded documents for external changes
  private async checkForExternalChanges(): Promise<void> {
    try {
      // Reload documents from filesystem first
      await this.storage.loadDocumentsFromFileSystem();

      // Check each document in memory for changes
      for (const docId of this.documents.keys()) {
        // Use a non-blocking approach to avoid locking everything during checks
        this.checkDocumentForExternalChanges(docId).catch(error => {
          this.log(
            'red',
            `Error checking for external changes to ${docId}: ${error.message}`,
          );
        });
      }
    } catch (error) {
      this.log(
        'red',
        `Error in checkForExternalChanges: ${(error as Error).message}`,
      );
    }
  }

  // Check a specific document for external changes
  private async checkDocumentForExternalChanges(
    docId: DocumentId,
  ): Promise<void> {
    // Skip if there's already an operation in progress for this document
    if (this.documentLocks.has(docId)) return;

    try {
      // Use the document lock to ensure thread safety
      await this.withDocumentLock(docId, async () => {
        // Get fresh copies of both documents to avoid aliasing issues
        const doc = this.documents.get(docId);
        if (!doc) return;

        // Get a fresh copy from storage
        const storedDoc = this.storage.getDocument(docId);
        if (!storedDoc) return;

        // Compare the stored doc with our in-memory version
        const docHistory = Automerge.getHistory(doc).length;
        const storedHistory = Automerge.getHistory(storedDoc).length;

        if (storedHistory > docHistory) {
          this.log(
            'blue',
            `Detected external changes to document: ${docId} (history: ${docHistory} -> ${storedHistory})`,
          );

          // Create a new merged document to avoid aliasing issues
          const merged = Automerge.merge(
            Automerge.clone(doc),
            Automerge.clone(storedDoc),
          );

          this.log('green', `Merged external changes to document: ${docId}`);
          this.documents.set(docId, merged);
          this.storage.storeDocument(docId, merged);
        }
      });
    } catch (error) {
      this.log(
        'red',
        `Error checking document ${docId} for external changes: ${
          (error as Error).message
        }`,
      );
    }
  }

  public async shutdown(): Promise<void> {
    this.stopFileWatcher();
    await this.storage.shutdown();
  }

  public getAllDocumentIds(): string[] {
    return Array.from(this.documents.keys());
  }

  private async withDocumentLock<T>(
    id: DocumentId,
    operation: () => Promise<T>,
  ): Promise<T> {
    // Wait for any existing operation to complete
    const existingLock = this.documentLocks.get(id);
    if (existingLock) {
      await existingLock;
    }

    // Create new lock
    let resolveLock: () => void;
    const newLock = new Promise<void>(resolve => {
      resolveLock = resolve;
    });
    this.documentLocks.set(id, newLock);

    try {
      return await operation();
    } finally {
      resolveLock!();
      this.documentLocks.delete(id);
    }
  }

  private async loadDocument(
    id: DocumentId,
    checkForExternalChanges: boolean = false,
  ): Promise<Automerge.Doc<any> | null> {
    let doc = this.documents.get(id);

    // If we need to check for external changes, or we don't have the doc in memory
    if (checkForExternalChanges || !doc) {
      // Get the latest version from storage
      const storedDoc = this.storage.getDocument(id);
      if (!storedDoc) return doc ? Automerge.clone(doc) : null;

      if (doc) {
        // If we already have a doc in memory, merge it with the stored version
        this.log('blue', `Checking for external changes to document: ${id}`);

        // Get history lengths for comparison
        const docHistory = Automerge.getHistory(doc).length;
        const storedHistory = Automerge.getHistory(storedDoc).length;

        // Only merge if there are actual changes to merge
        if (storedHistory > docHistory) {
          // Create clones to avoid aliasing issues
          const docClone = Automerge.clone(doc);
          const storedClone = Automerge.clone(storedDoc);

          // Merge the two documents
          const merged = Automerge.merge(docClone, storedClone);

          this.log(
            'green',
            `Merged external changes to document: ${id} (history: ${docHistory} -> ${storedHistory})`,
          );
          this.documents.set(id, merged);
          this.storage.storeDocument(id, merged); // Save the merged result
          return Automerge.clone(merged);
        }
      } else {
        // First time loading this doc
        doc = Automerge.clone(storedDoc); // Clone to avoid aliasing issues
        this.documents.set(id, doc);
      }
    }

    return doc ? Automerge.clone(doc) : null;
  }

  public async createDocument(
    id: DocumentId,
    initialContent: any = {},
  ): Promise<void> {
    return this.withDocumentLock(id, async () => {
      const doc = Automerge.init();
      const newDoc = Automerge.change(doc, (d: any) => {
        Object.assign(d, initialContent);
      });

      this.documents.set(id, newDoc);
      this.storage.storeDocument(id, newDoc);

      this.log('blue', `Created new document: ${id}`);
    });
  }

  public async getDocument(
    id: DocumentId,
    checkForExternalChanges: boolean = true,
  ): Promise<Automerge.Doc<any> | null> {
    const doc = await this.loadDocument(id, checkForExternalChanges);
    if (!doc) return null;
    return doc;
  }

  public async updateDocument(
    id: DocumentId,
    updater: (doc: any) => void,
  ): Promise<void> {
    return this.withDocumentLock(id, async () => {
      // Always check for external changes before updating
      const doc = await this.loadDocument(id, true);
      if (!doc) {
        // Create a new document if it doesn't exist
        return this.createDocument(id, {});
      }

      try {
        const newDoc = Automerge.change(doc, updater);
        this.documents.set(id, newDoc);
        this.storage.storeDocument(id, newDoc);

        this.log('blue', `Document updated: ${id}`);
      } catch (error) {
        this.log(
          'red',
          `Error updating document ${id}: ${(error as Error).message}`,
        );
        throw error;
      }
    });
  }

  public async handleMessage(
    message: Buffer,
  ): Promise<{patch: any | null; docId: string; didChange: boolean} | null> {
    try {
      // Try to parse the message
      const data = JSON.parse(message.toString());

      if (!data.docId || !data.changes) {
        return null; // No document ID or changes, can't process
      }

      this.log('blue', `Processing changes for document: ${data.docId}`);
      return this.handleIncomingChanges(data.docId, data.changes);
    } catch (error) {
      // Not a JSON message or doesn't contain the expected fields
      // This is normal for binary messages or other protocols
      return null;
    }
  }

  public async handleIncomingChanges(
    docId: string,
    changes: Uint8Array | number[],
  ): Promise<{patch: any | null; docId: string; didChange: boolean}> {
    return this.withDocumentLock(docId, async () => {
      this.log('blue', `Loading document of id ${docId}`);

      // Always check for external changes when handling incoming changes
      let doc = await this.loadDocument(docId, true);
      if (!doc) {
        this.log('blue', `Initializing a new document of id ${docId}`);
        doc = Automerge.init();
        this.documents.set(docId, doc);
      }

      doc = Automerge.clone(doc);
      const syncState = this.syncStates.get(docId) || Automerge.initSyncState();

      try {
        this.log('blue', `Receiving sync message for: ${docId}`);

        // Convert the changes to Uint8Array if they're an array of numbers
        const changesUint8 =
          changes instanceof Uint8Array ? changes : new Uint8Array(changes);

        const [newDoc, newSyncState, patch] = Automerge.receiveSyncMessage(
          doc,
          syncState,
          changesUint8,
        );

        logger.info('NEW DOC:\n');
        printFirstEntries(newDoc);

        this.documents.set(docId, newDoc);
        this.syncStates.set(docId, newSyncState);
        this.storage.storeDocument(docId, newDoc);

        if (patch) {
          this.log('blue', `Received patch for ${docId}`);
          return {patch, docId, didChange: true};
        }

        return {patch: null, docId, didChange: false};
      } catch (error) {
        this.log(
          'red',
          `Error handling changes for ${docId}: ${(error as Error).message}`,
        );
        throw error;
      }
    });
  }

  public async generateSyncMessage(
    docId: DocumentId,
  ): Promise<Uint8Array | null> {
    const doc = this.documents.get(docId);
    if (!doc) {
      this.log('yellow', `No document found for id: ${docId}`);
      return null;
    }

    const syncState = this.syncStates.get(docId) || Automerge.initSyncState();
    const [newSyncState, message] = Automerge.generateSyncMessage(
      doc,
      syncState,
    );

    if (message) {
      this.log('blue', `Generated sync message for ${docId}`);
      this.syncStates.set(docId, newSyncState);
      return message;
    }

    return null;
  }

  // Returns serialized document for sending to clients
  public getSerializedDocument(docId: string): Buffer | null {
    const doc = this.documents.get(docId);
    if (!doc) return null;

    // Serialize the document to send to the client
    const serialized = Automerge.save(doc);
    return Buffer.from(serialized);
  }

  // Force an immediate sync to the filesystem
  public async forceSyncToFileSystem(): Promise<void> {
    return this.storage.forceSyncToFileSystem();
  }

  // Reload documents from the filesystem
  public async reloadFromFileSystem(): Promise<void> {
    await this.storage.loadDocumentsFromFileSystem();

    // Update our in-memory state with the freshly loaded documents
    for (const docId of this.storage.getAllDocumentIds()) {
      // Use a try-catch block for each document to prevent one failure from stopping the entire reload
      try {
        const storedDoc = this.storage.getDocument(docId);
        if (!storedDoc) continue;

        const inMemoryDoc = this.documents.get(docId);

        if (inMemoryDoc) {
          // If we have the document in memory, merge it with the stored version
          // Get history lengths for comparison
          const docHistory = Automerge.getHistory(inMemoryDoc).length;
          const storedHistory = Automerge.getHistory(storedDoc).length;

          if (storedHistory > docHistory) {
            this.log(
              'blue',
              `Merging changes for document: ${docId} during reload (history: ${docHistory} -> ${storedHistory})`,
            );

            // Create clones to avoid aliasing issues
            const docClone = Automerge.clone(inMemoryDoc);
            const storedClone = Automerge.clone(storedDoc);

            // Merge the two documents
            const merged = Automerge.merge(docClone, storedClone);
            this.documents.set(docId, merged);
          }
        } else {
          // If we don't have the document in memory, just use the stored version
          this.documents.set(docId, Automerge.clone(storedDoc));
        }
      } catch (error) {
        this.log(
          'red',
          `Error reloading document ${docId}: ${(error as Error).message}`,
        );
      }
    }

    this.log('green', 'Reloaded documents from filesystem');
  }

  // Delete a document
  public async deleteDocument(id: DocumentId): Promise<void> {
    return this.withDocumentLock(id, async () => {
      this.documents.delete(id);
      this.syncStates.delete(id);
      await this.storage.deleteDocument(id);
      this.log('blue', `Document ${id} deleted`);
    });
  }

  // Clear memory cache
  public clear(): void {
    this.documents.clear();
    this.syncStates.clear();
    this.documentLocks.clear();
  }

  // Manually check for external changes to a specific document
  public async checkForExternalChangesTo(docId: DocumentId): Promise<boolean> {
    try {
      // Reload documents from filesystem first
      await this.storage.loadDocumentsFromFileSystem();

      let hasChanges = false;

      // Use the document lock to ensure thread safety
      await this.withDocumentLock(docId, async () => {
        // Get fresh copies of both documents to avoid aliasing issues
        const doc = this.documents.get(docId);
        if (!doc) return;

        // Get a fresh copy from storage
        const storedDoc = this.storage.getDocument(docId);
        if (!storedDoc) return;

        // Compare the stored doc with our in-memory version
        const docHistory = Automerge.getHistory(doc).length;
        const storedHistory = Automerge.getHistory(storedDoc).length;

        if (storedHistory > docHistory) {
          this.log(
            'blue',
            `Detected external changes to document: ${docId} (history: ${docHistory} -> ${storedHistory})`,
          );

          // Create a new merged document to avoid aliasing issues
          const docClone = Automerge.clone(doc);
          const storedClone = Automerge.clone(storedDoc);
          const merged = Automerge.merge(docClone, storedClone);

          this.documents.set(docId, merged);
          this.storage.storeDocument(docId, merged);

          this.log(
            'green',
            `Manually merged external changes to document: ${docId}`,
          );
          hasChanges = true;
        }
      });

      return hasChanges;
    } catch (error) {
      this.log(
        'red',
        `Error checking for external changes to ${docId}: ${
          (error as Error).message
        }`,
      );
      return false;
    }
  }
}

import {
  AutomergeBackblazeStorage,
  BackblazeStorageMiddlewareOptions,
} from './backblazeStorage.js';
import * as Automerge from '@automerge/automerge';
import {DocumentId} from './types.js';
import {logger} from './logger.js';

export class BackblazeStorageMiddleware {
  private storage: AutomergeBackblazeStorage;
  private documents: Map<DocumentId, Automerge.Doc<any>> = new Map();
  private syncStates: Map<DocumentId, Automerge.SyncState> = new Map();
  private documentLocks: Map<DocumentId, Promise<void>> = new Map();
  private name: string;

  constructor(
    options: BackblazeStorageMiddlewareOptions,
    name: string = 'StorageMiddleware',
    private verbose: boolean = true,
  ) {
    this.name = name;
    this.storage = new AutomergeBackblazeStorage(
      options,
      (color, message) => this.log(color, message),
      verbose,
    );
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

  public async shutdown(): Promise<void> {
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
  ): Promise<Automerge.Doc<any> | null> {
    let doc = this.documents.get(id);
    if (doc) return Automerge.clone(doc);

    const storedDoc = this.storage.getDocument(id);
    if (!storedDoc) return null;

    doc = storedDoc; // Already an Automerge.Doc from storage.getDocument
    this.documents.set(id, doc);
    return Automerge.clone(doc);
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

  public async getDocument(id: DocumentId): Promise<Automerge.Doc<any> | null> {
    const doc = await this.loadDocument(id);
    if (!doc) return null;
    return doc;
  }

  public async updateDocument(
    id: DocumentId,
    updater: (doc: any) => void,
  ): Promise<void> {
    return this.withDocumentLock(id, async () => {
      const doc = await this.loadDocument(id);
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

      let doc = await this.loadDocument(docId);
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

  // Force an immediate sync to Backblaze
  public async forceSyncToBackblaze(): Promise<void> {
    return this.storage.forceSyncToBackblaze();
  }

  // Reload documents from Backblaze
  public async reloadFromBackblaze(): Promise<void> {
    await this.storage.loadDocumentsFromBackblaze();

    // Update our in-memory state with the freshly loaded documents
    for (const docId of this.storage.getAllDocumentIds()) {
      const doc = this.storage.getDocument(docId);
      if (doc) {
        this.documents.set(docId, Automerge.clone(doc));
      }
    }
  }

  // Clear memory cache
  public clear(): void {
    this.documents.clear();
    this.syncStates.clear();
    this.documentLocks.clear();
  }
}

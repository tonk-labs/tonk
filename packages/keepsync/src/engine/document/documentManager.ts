import * as Automerge from '@automerge/automerge';
import {DocumentId, DocumentData} from '../types';
import {StorageProvider} from '../types';
import {logger} from '../../utils/logger';

export class DocumentManager {
  private documents: Map<DocumentId, Automerge.Doc<any>> = new Map();
  private syncStates: Map<DocumentId, Automerge.SyncState> = new Map();
  private documentLocks: Map<DocumentId, Promise<void>> = new Map();
  private name: string;

  constructor(
    private storage: StorageProvider,
    name: string = 'DocumentManager',
  ) {
    this.name = name;
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

  async loadDocument(id: DocumentId): Promise<Automerge.Doc<any> | null> {
    let doc = this.documents.get(id);
    if (doc) return Automerge.clone(doc);

    const docBytes = await this.storage.getDocument(id);
    if (!docBytes) return null;

    doc = Automerge.load(docBytes);
    this.documents.set(id, doc);
    return Automerge.clone(doc);
  }

  async createDocument(
    id: DocumentId,
    initialContent: DocumentData = {},
  ): Promise<void> {
    const doc = Automerge.init();
    const newDoc = Automerge.change(doc, (d: any) => {
      Object.assign(d, initialContent);
    });

    this.documents.set(id, newDoc);
    await this.storage.saveDocument(id, Automerge.save(newDoc));
  }

  async getDocument(id: DocumentId): Promise<any | null> {
    const doc = await this.loadDocument(id);
    if (!doc) return null;
    return doc;
  }

  async updateDocument(
    id: DocumentId,
    updater: (doc: any) => void,
  ): Promise<void> {
    await this.withDocumentLock(id, async () => {
      const doc = await this.loadDocument(id);
      if (!doc) throw new Error('Document not found');

      try {
        const newDoc = Automerge.change(doc, updater);
        this.documents.set(id, newDoc);
        await this.storage.saveDocument(id, Automerge.save(newDoc));

        logger.debugWithContext(this.name, `Document updated: ${id}`);
      } catch (error) {
        logger.error(`Error updating document ${id}:`, error);
        throw error;
      }
    });
  }

  async handleIncomingChanges(
    docId: string,
    changes: number[],
  ): Promise<{patch: any | null; didChange: boolean}> {
    logger.debugWithContext(this.name, `Loading document again of id ${docId}`);

    let doc = await this.loadDocument(docId);
    if (!doc) {
      logger.debugWithContext(
        this.name,
        `Initializing a new document of id ${docId}`,
      );
      doc = Automerge.init();
      this.documents.set(docId, doc);
    }

    doc = Automerge.clone(doc);
    const syncState = this.syncStates.get(docId) || Automerge.initSyncState();

    try {
      logger.debugWithContext(this.name, 'Receiving sync message for:', {
        docId,
        docContent: doc,
      });

      const [newDoc, newSyncState, patch] = Automerge.receiveSyncMessage(
        doc,
        syncState,
        new Uint8Array(changes),
      );

      logger.debugWithContext(this.name, 'After receiving sync message:', {
        docId,
        newDocContent: newDoc,
      });

      this.documents.set(docId, newDoc);
      this.syncStates.set(docId, newSyncState);
      await this.storage.saveDocument(docId, Automerge.save(newDoc));

      if (patch) {
        logger.debugWithContext(
          this.name,
          `Received patch for ${docId}:`,
          patch,
        );
        return {patch, didChange: true};
      }

      return {patch: null, didChange: false};
    } catch (error) {
      logger.error(`Error handling changes for ${docId}:`, error);
      throw error;
    }
  }

  async generateSyncMessage(docId: DocumentId): Promise<Uint8Array | null> {
    const doc = this.documents.get(docId);
    if (!doc) {
      logger.warn(`No document found for id: ${docId}`);
      return null;
    }

    const syncState = this.syncStates.get(docId) || Automerge.initSyncState();
    const [newSyncState, message] = Automerge.generateSyncMessage(
      doc,
      syncState,
    );

    if (message) {
      logger.debugWithContext(
        this.name,
        `Generated sync message for ${docId}:`,
        {
          messageLength: message.length,
          docContent: doc,
        },
      );

      this.syncStates.set(docId, newSyncState);
      return message;
    }

    return null;
  }

  getAllDocumentIds(): string[] {
    return Array.from(this.documents.keys());
  }

  clear(): void {
    this.documents.clear();
    this.syncStates.clear();
    this.documentLocks.clear();
  }
}

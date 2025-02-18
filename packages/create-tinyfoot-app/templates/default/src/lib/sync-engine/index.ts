import { Storage } from "./storage";
import { DocumentId, SyncEngineOptions } from "./types";
import * as Automerge from '@automerge/automerge';

export class SyncEngine {
  private storage: Storage;
  private ws: WebSocket | null = null;
  private syncStates: Map<DocumentId, Automerge.SyncState> = new Map();
  private documents: Map<DocumentId, Automerge.Doc<any>> = new Map();
  private initialised = false;

  constructor(private options: SyncEngineOptions = {}) {
    this.storage = new Storage(options.dbName);
  }

  async init(): Promise<void> {
    if (this.initialised) return;

    try {
      await this.storage.init();
      this.connectWebSocket(`ws://localhost:${this.options.port || 9000}/sync`);
      this.initialised = true;
    } catch (error) {
      console.error('Failed to initialise SyncEngine:', error);
      throw error;
    }
  }

  private connectWebSocket(url: string): void {
    this.ws = new WebSocket(url);

    this.ws.onopen = async () => {
      // When connection is established, sync all documents
      for (const docId of this.documents.keys()) {
        await this.sendChanges(docId);
      }
    };

    this.ws.onmessage = async (event) => {
      try {
        const { docId, changes } = JSON.parse(event.data);
        console.log(`Received changes for ${docId}:`, changes);
        await this.handleIncomingChanges(docId, changes);
      } catch (error) {
        console.error('WebSocket message error:', error);
        this.options.onError?.(error as Error);
      }
    };

    this.ws.onerror = (error) => {
      console.error('WebSocket error:', error);
      this.options.onError?.(error as unknown as Error);
    }

    this.ws.onclose = () => {
      console.log('WebSocket closed, attempting to reconnect...');
      setTimeout(() => this.connectWebSocket(url), 1000);
    }
  }

  private async handleIncomingChanges(docId: string, changes: number[]): Promise<void> {
    const doc = await this.loadDocument(docId);
    if (!doc) {
      console.warn(`No document found for id: ${docId}`);
      return;
    }

    let syncState = this.syncStates.get(docId) || Automerge.initSyncState();

    try {
      const [newDoc, newSyncState, patch] = Automerge.receiveSyncMessage(
        doc,
        syncState,
        new Uint8Array(changes)
      );

      if (patch) {
        console.log(`Received patch for ${docId}:`, patch);
        this.documents.set(docId, newDoc);
        this.syncStates.set(docId, newSyncState);
        await this.storage.saveDocument(docId, Automerge.save(newDoc));

        // Send any pending changes back to ensure bidirectional sync
        await this.sendChanges(docId);

        this.options.onSync?.(docId);
      }
    } catch (error) {
      console.error(`Error handling changes for ${docId}:`, error);
      this.options.onError?.(error as Error);
    }
  }

  private async loadDocument(id: DocumentId): Promise<Automerge.Doc<any> | null> {
    const loadedDoc = this.documents.get(id);
    if (loadedDoc) return loadedDoc;

    const docBytes = await this.storage.getDocument(id);
    if (!docBytes) return null;

    const doc = Automerge.load(docBytes);
    this.documents.set(id, doc);
    return doc;
  }

  async createDocument(id: DocumentId, initialContent: any = {}): Promise<void> {
    if (!this.initialised) {
      throw new Error('SyncEngine not initialised');
    }

    const doc = Automerge.init();
    const newDoc = Automerge.change(doc, (d: any) => {
      Object.assign(d, initialContent);
    });

    this.documents.set(id, newDoc);
    await this.storage.saveDocument(id, Automerge.save(newDoc));
    await this.sendChanges(id);
  }

  async getDocument(id: DocumentId): Promise<any | null> {
    const doc = await this.loadDocument(id);
    if (!doc) return null;
    return doc;
  }

  async updateDocument(id: DocumentId, updater: (doc: any) => void): Promise<void> {
    if (!this.initialised) {
      throw new Error('SyncEngine not initialised');
    }

    const doc = await this.loadDocument(id);
    if (!doc) throw new Error('Document not found');

    const newDoc = Automerge.change(doc, updater);
    this.documents.set(id, newDoc);
    await this.storage.saveDocument(id, Automerge.save(newDoc));
    await this.sendChanges(id);

    console.log(`Document updated: ${id}`);
  }

  private async sendChanges(docId: DocumentId): Promise<void> {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      console.warn('WebSocket not connected');
      return;
    }

    const doc = this.documents.get(docId);
    if (!doc) {
      console.warn(`No document found for id: ${docId}`);
      return;
    }

    let syncState = this.syncStates.get(docId) || Automerge.initSyncState();
    const [newSyncState, message] = Automerge.generateSyncMessage(doc, syncState);

    if (message) {
      console.log(`Sending changes for ${docId}:`, Array.from(message));
      this.syncStates.set(docId, newSyncState);
      this.ws.send(JSON.stringify({
        docId,
        changes: Array.from(message)
      }));
    }
  }

  close(): void {
    this.ws?.close();
  }
}

// Example usage:
/*
// Start a sync engine with integrated server
const engine1 = new SyncEngine({
  port: 9000, // Use your Express server port
  onSync: (docId) => console.log(`Document ${docId} synced`),
  onError: (error) => console.error('Sync error:', error)
});
await engine1.init()

// Create and update documents
await engine1.createDocument('doc1', { content: 'Hello' })
await engine1.updateDocument('doc1', doc => {
  doc.content = 'Hello, World!'
})

// Save and retrieve blobs
const blob = new Blob(['Hello'], { type: 'text/plain' })
await engine1.saveBlob('blob1', blob)
*/

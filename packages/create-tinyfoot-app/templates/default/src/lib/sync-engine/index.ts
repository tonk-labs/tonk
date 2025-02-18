import { SyncManager } from "./sync";
import { Storage } from "./storage";
import { BlobId, Document, DocumentId, SyncEngineOptions } from "./types";
import * as Automerge from '@automerge/automerge';

export class SyncEngine {
  private storage: Storage;
  private syncManager: SyncManager;

  constructor(options: SyncEngineOptions) {
    this.storage = new Storage(options.dbName);
    this.syncManager = new SyncManager(
      options.websocketUrl,
      this.storage,
      options.onSync,
      options.onError
    );
  }

  async init(): Promise<void> {
    await this.storage.init();
    this.syncManager.connect();
  }

  async createDocument(id: DocumentId, initialContent: any = {}): Promise<void> {
    const doc = Automerge.init();
    const newDoc = Automerge.change(doc, (d: any) => {
      Object.assign(d, initialContent);
    });

    await this.storage.saveDocument(id, Automerge.save(newDoc));
    await this.syncManager.sendChanges(id);
  }

  async getDocument(id: DocumentId): Promise<Document | null> {
    const docBytes = await this.storage.getDocument(id);
    if (!docBytes) return null;

    const doc = Automerge.load(docBytes);
    return {
      content: doc,
      blobs: {}
    };
  }

  async updateDocument(id: DocumentId, updater: (doc: any) => void): Promise<void> {
    const docBytes = await this.storage.getDocument(id);
    if (!docBytes) throw new Error('Document not found');

    const doc = Automerge.load(docBytes);
    const newDoc = Automerge.change(doc, updater);

    await this.storage.saveDocument(id, Automerge.save(newDoc));
    await this.syncManager.sendChanges(id);
  }

  async saveBlob(id: BlobId, blob: Blob): Promise<void> {
    await this.storage.saveBlob(id, blob);
  }

  async getBlob(id: BlobId): Promise<Blob | null> {
    return await this.storage.getBlob(id);
  }
}

// Example usage:
/*
const syncEngine = new SyncEngine({
  websocketUrl: 'ws://localhost:8080',
  dbName: 'my-app-store',
  onSync: (docId) => console.log(`Document ${docId} synced`),
  onError: (error) => console.error('Sync error:', error)
})

await syncEngine.init()

// Create a new document
await syncEngine.createDocument('doc1', { 
  title: 'My Document',
  content: 'Hello, world!'
})

// Update document
await syncEngine.updateDocument('doc1', (doc) => {
  doc.content = 'Updated content'
})

// Get document
const doc = await syncEngine.getDocument('doc1')

// Save blob
const blob = new Blob(['Hello, world!'], { type: 'text/plain' })
await syncEngine.saveBlob('blob1', blob)

// Get blob
const retrievedBlob = await syncEngine.getBlob('blob1')
*/

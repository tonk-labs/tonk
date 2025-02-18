import { Storage } from "./storage";
import { SyncServer } from "./server";
import { BlobId, Document, DocumentId, SyncEngineOptions } from "./types";
import * as Automerge from '@automerge/automerge';

export class SyncEngine {
  private storage: Storage;
  private ws: WebSocket | null = null;
  private server: SyncServer | null = null;
  private syncStates: Map<DocumentId, Automerge.SyncState> = new Map();
  private documents: Map<DocumentId, Automerge.Doc<any>> = new Map();

  constructor(private options: SyncEngineOptions = {}) {
    this.storage = new Storage(options.dbName);
  }

  async init(): Promise<void> {
    await this.storage.init();

    if (this.options.port) {
      this.server = new SyncServer(this.options.port);
      this.connectWebSocket(`ws://localhost:${this.options.port}`);
    }
  }

  private connectWebSocket(url: string): void {
    this.ws = new WebSocket(url);

    this.ws.onmessage = async (event) => {
      try {
        const { docId, changes } = JSON.parse(event.data);
        const doc = this.documents.get(docId);

        if (doc && changes) {
          let syncState = this.syncStates.get(docId) || Automerge.initSyncState();
          const [newDoc, newSyncState, patch] = Automerge.receiveSyncMessage(
            doc,
            syncState,
            new Uint8Array(changes)
          );

          if (patch) {
            this.documents.set(docId, newDoc);
            this.syncStates.set(docId, newSyncState);
            await this.storage.saveDocument(docId, Automerge.save(newDoc));
            this.options.onSync?.(docId);
          }
        }
      } catch (error) {
        this.options.onError?.(error as Error);
      }
    }

    this.ws.onerror = (error) => {
      this.options.onError?.(error as unknown as Error);
    }
  }

  async createDocument(id: DocumentId, initialContent: any = {}): Promise<void> {
    const doc = Automerge.init();
    const newDoc = Automerge.change(doc, (d: any) => {
      Object.assign(d, initialContent);
    });

    this.documents.set(id, newDoc);
    await this.storage.saveDocument(id, Automerge.save(newDoc));
    await this.sendChanges(id);
  }

  async getDocument(id: DocumentId): Promise<Document | null> {
    const docBytes = await this.storage.getDocument(id);
    if (!docBytes) return null;

    const doc = Automerge.load(docBytes);
    this.documents.set(id, doc);
    return {
      content: doc,
      blobs: {}
    };
  }

  async updateDocument(id: DocumentId, updater: (doc: any) => void): Promise<void> {
    const docBytes = this.documents.get(id);
    if (!docBytes) throw new Error('Document not found');

    const newDoc = Automerge.change(docBytes, updater);
    this.documents.set(id, newDoc);
    await this.storage.saveDocument(id, Automerge.save(newDoc));
    await this.sendChanges(id);
  }

  private async sendChanges(docId: DocumentId): Promise<void> {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;

    const doc = this.documents.get(docId);
    if (!doc) return;

    let syncState = this.syncStates.get(docId) || Automerge.initSyncState();
    const [newSyncState, message] = Automerge.generateSyncMessage(doc, syncState);

    if (message) {
      this.syncStates.set(docId, newSyncState);
      this.ws.send(JSON.stringify({
        docId,
        changes: Array.from(message)
      }));
    }
  }

  async saveBlob(id: BlobId, blob: Blob): Promise<void> {
    await this.storage.saveBlob(id, blob);
  }

  async getBlob(id: BlobId): Promise<Blob | null> {
    return await this.storage.getBlob(id);
  }

  close(): void {
    this.server?.close();
  }
}

// Example usage:
/*
// Start a sync engine with integrated server
const engine1 = new SyncEngine({ port: 8080 })
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

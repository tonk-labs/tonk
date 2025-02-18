import { BlobId, DocumentId } from "./types";

export class Storage {
  private db: IDBDatabase | null = null;
  private readonly DB_NAME: string;
  private readonly DOCS_STORE = 'documents';
  private readonly BLOBS_STORE = 'blobs';

  constructor(dbName: string = 'sync-engine-store') {
    this.DB_NAME = dbName;
  }

  async init(): Promise<void> {
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(this.DB_NAME, 1);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => {
        this.db = request.result;
        resolve();
      };

      request.onupgradeneeded = () => {
        const db = request.result;
        db.createObjectStore(this.DOCS_STORE);
        db.createObjectStore(this.BLOBS_STORE);
      };
    });
  }

  async saveDocument(id: DocumentId, doc: Uint8Array): Promise<void> {
    if (!this.db) throw new Error('Database not initialised');

    return new Promise((resolve, reject) => {
      const tx = this.db!.transaction(this.DOCS_STORE, 'readwrite');
      const store = tx.objectStore(this.DOCS_STORE);
      const request = store.put(doc, id);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve();
    });
  }

  async saveBlob(id: BlobId, blob: Blob): Promise<void> {
    if (!this.db) throw new Error('Database not initialised');

    return new Promise((resolve, reject) => {
      const tx = this.db!.transaction(this.BLOBS_STORE, 'readwrite');
      const store = tx.objectStore(this.BLOBS_STORE);
      const request = store.put(blob, id);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve();
    });
  }

  async getDocument(id: DocumentId): Promise<Uint8Array | null> {
    if (!this.db) throw new Error('Database not initialised');

    return new Promise((resolve, reject) => {
      const tx = this.db!.transaction(this.DOCS_STORE, 'readwrite');
      const store = tx.objectStore(this.DOCS_STORE);
      const request = store.get(id);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve(request.result);
    });
  }

  async getBlob(id: BlobId): Promise<Blob | null> {
    if (!this.db) throw new Error('Database not initialised');

    return new Promise((resolve, reject) => {
      const tx = this.db!.transaction(this.BLOBS_STORE, 'readwrite');
      const store = tx.objectStore(this.BLOBS_STORE);
      const request = store.get(id);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve(request.result);
    });
  }
}

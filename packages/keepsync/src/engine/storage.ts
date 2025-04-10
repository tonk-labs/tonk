import {BlobId, DocumentId} from './types';

const hasElectronIPC = (): boolean => {
  return (
    typeof window !== 'undefined' &&
    window.electronAPI !== undefined &&
    typeof window.electronAPI.writeDocument === 'function' &&
    typeof window.electronAPI.readDocument === 'function'
  );
};

export class Storage {
  private db: IDBDatabase | null = null;
  private dbInitPromise: Promise<void> | null = null;
  private readonly DB_NAME: string;
  private readonly DOCS_STORE = 'documents';
  private readonly BLOBS_STORE = 'blobs';
  private readonly hasElectronIPC: boolean;

  constructor(dbName = 'sync-engine-store') {
    this.DB_NAME = dbName;
    this.hasElectronIPC = hasElectronIPC();
  }

  async init(): Promise<void> {
    if (this.dbInitPromise) {
      return this.dbInitPromise;
    }

    this.dbInitPromise = new Promise((resolve, reject) => {
      const request = indexedDB.open(this.DB_NAME, 1);

      request.onerror = () => {
        console.error('Failed to open database:', request.error);
        reject(request.error);
      };

      request.onsuccess = () => {
        this.db = request.result;
        resolve();
      };

      request.onupgradeneeded = () => {
        const db = request.result;

        if (!db.objectStoreNames.contains(this.DOCS_STORE)) {
          db.createObjectStore(this.DOCS_STORE);
        }

        if (!db.objectStoreNames.contains(this.BLOBS_STORE)) {
          db.createObjectStore(this.BLOBS_STORE);
        }
      };
    });

    return this.dbInitPromise;
  }

  private async ensureDB(): Promise<IDBDatabase> {
    if (!this.db) {
      await this.init();
      if (!this.db) throw new Error('Database failed to initialize');
    }

    return this.db;
  }

  async saveDocument(id: DocumentId, doc: Uint8Array): Promise<void> {
    if (this.hasElectronIPC) await window.electronAPI.writeDocument(id, doc);

    const db = await this.ensureDB();

    return new Promise((resolve, reject) => {
      const tx = db.transaction(this.DOCS_STORE, 'readwrite');
      const store = tx.objectStore(this.DOCS_STORE);
      const request = store.put(doc, id);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve();
    });
  }

  async saveBlob(id: BlobId, blob: Blob): Promise<void> {
    const db = await this.ensureDB();

    return new Promise((resolve, reject) => {
      const tx = db.transaction(this.BLOBS_STORE, 'readwrite');
      const store = tx.objectStore(this.BLOBS_STORE);
      const request = store.put(blob, id);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve();
    });
  }

  async getDocument(id: DocumentId): Promise<Uint8Array | null> {
    if (this.hasElectronIPC) {
      try {
        const result = await window.electronAPI.readDocument(id);
        return result;
      } catch (error) {
        console.error('Failed to read document via Electron IPC:', error);
        // Fall back to IndexedDB if Electron IPC fails
      }
    }

    // Fall back to IndexedDB
    const db = await this.ensureDB();

    return new Promise((resolve, reject) => {
      const tx = db.transaction(this.DOCS_STORE, 'readwrite');
      const store = tx.objectStore(this.DOCS_STORE);
      const request = store.get(id);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve(request.result);
    });
  }

  async getBlob(id: BlobId): Promise<Blob | null> {
    const db = await this.ensureDB();

    return new Promise((resolve, reject) => {
      const tx = db.transaction(this.BLOBS_STORE, 'readwrite');
      const store = tx.objectStore(this.BLOBS_STORE);
      const request = store.get(id);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve(request.result);
    });
  }
}

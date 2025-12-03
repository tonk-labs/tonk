import type { Bundle, BundleData } from '../types';

const DB_NAME = 'tonk-launcher';
const DB_VERSION = 1;
const STORE_NAME = 'bundles';

export class BundleStorage {
  private dbPromise: Promise<IDBDatabase> | null = null;

  constructor() {
    this.dbPromise = this.openDB();
  }

  private openDB(): Promise<IDBDatabase> {
    return new Promise((resolve, reject) => {
      if (typeof indexedDB === 'undefined') {
        reject(new Error('IndexedDB is not available in this environment.'));
        return;
      }

      const request = indexedDB.open(DB_NAME, DB_VERSION);

      request.onerror = () => {
        reject(new Error(`Failed to open database: ${request.error?.message}`));
      };

      request.onsuccess = () => {
        resolve(request.result);
      };

      request.onupgradeneeded = (event) => {
        const db = (event.target as IDBOpenDBRequest).result;
        if (!db.objectStoreNames.contains(STORE_NAME)) {
          db.createObjectStore(STORE_NAME, { keyPath: 'id' });
        }
      };
    });
  }

  private async getDB(): Promise<IDBDatabase> {
    if (!this.dbPromise) {
      this.dbPromise = this.openDB();
    }
    return this.dbPromise;
  }

  /**
   * Save a bundle to storage.
   * @param id - The unique identifier for the bundle.
   * @param data - The bundle data (excluding id and createdAt, which are handled automatically if not provided, though the interface expects full data mostly).
   *               Actually, the interface in docs says save(id, data: Omit<BundleData, 'id' | 'createdAt'>).
   */
  async save(id: string, data: Omit<BundleData, 'id' | 'createdAt'>): Promise<void> {
    const db = await this.getDB();
    return new Promise((resolve, reject) => {
      const transaction = db.transaction([STORE_NAME], 'readwrite');
      const store = transaction.objectStore(STORE_NAME);

      const bundle: BundleData = {
        id,
        createdAt: Date.now(),
        ...data,
      };

      const request = store.put(bundle);

      request.onerror = () => {
        reject(new Error(`Failed to save bundle: ${request.error?.message}`));
      };

      request.onsuccess = () => {
        resolve();
      };
    });
  }

  /**
   * Retrieve a bundle by its ID.
   * @param id - The bundle ID.
   * @returns The bundle data or null if not found.
   */
  async get(id: string): Promise<BundleData | null> {
    const db = await this.getDB();
    return new Promise((resolve, reject) => {
      const transaction = db.transaction([STORE_NAME], 'readonly');
      const store = transaction.objectStore(STORE_NAME);
      const request = store.get(id);

      request.onerror = () => {
        reject(new Error(`Failed to get bundle: ${request.error?.message}`));
      };

      request.onsuccess = () => {
        resolve(request.result || null);
      };
    });
  }

  /**
   * List all bundles (metadata only).
   * Uses a cursor to iterate over the store, avoiding loading all binary data into memory at once.
   */
  async list(): Promise<Bundle[]> {
    const db = await this.getDB();
    return new Promise((resolve, reject) => {
      const transaction = db.transaction([STORE_NAME], 'readonly');
      const store = transaction.objectStore(STORE_NAME);
      const results: Bundle[] = [];
      const request = store.openCursor();

      request.onerror = () => {
        reject(new Error(`Failed to list bundles: ${request.error?.message}`));
      };

      request.onsuccess = (event) => {
        const cursor = (event.target as IDBRequest<IDBCursorWithValue | null>).result;
        if (!cursor) {
          resolve(results);
          return;
        }

        const b = cursor.value as BundleData;
        results.push({
          id: b.id,
          name: b.name,
          size: b.size,
          createdAt: b.createdAt,
        });
        cursor.continue();
      };
    });
  }

  /**
   * Delete a bundle by its ID.
   * @param id - The bundle ID.
   */
  async delete(id: string): Promise<void> {
    const db = await this.getDB();
    return new Promise((resolve, reject) => {
      const transaction = db.transaction([STORE_NAME], 'readwrite');
      const store = transaction.objectStore(STORE_NAME);
      const request = store.delete(id);

      request.onerror = () => {
        reject(new Error(`Failed to delete bundle: ${request.error?.message}`));
      };

      request.onsuccess = () => {
        resolve();
      };
    });
  }
}

export const bundleStorage = new BundleStorage();

import {FileManagerOptions, FileMetadata} from './types.js';

export class FileManager {
  private dbName: string;
  private storeName: string;
  private db: IDBDatabase | null = null;
  private dbInitPromise: Promise<void> | null = null;

  constructor(options: FileManagerOptions = {}) {
    this.dbName = options.dbName || 'sync-engine-store';
    this.storeName = options.storeName || 'file-blobs';
  }

  /**
   * Initialize the database
   */
  async init(): Promise<void> {
    if (this.db) return;
    if (this.dbInitPromise) return this.dbInitPromise;

    this.dbInitPromise = new Promise((resolve, reject) => {
      const request = indexedDB.open(this.dbName, 1);

      request.onerror = () => {
        console.error('Failed to open database:', request.error);
        reject(new Error('Failed to open database'));
      };

      request.onsuccess = () => {
        this.db = request.result;

        // Check if our store exists, if not we need to create it in a version change
        if (!this.db.objectStoreNames.contains(this.storeName)) {
          this.db.close();
          // Get current version and increment it
          const newVersion = this.db.version + 1;
          const reopenRequest = indexedDB.open(this.dbName, newVersion);

          reopenRequest.onupgradeneeded = () => {
            const db = reopenRequest.result;
            if (!db.objectStoreNames.contains(this.storeName)) {
              db.createObjectStore(this.storeName);
            }
          };

          reopenRequest.onsuccess = () => {
            this.db = reopenRequest.result;
            resolve();
          };

          reopenRequest.onerror = () => {
            console.error('Failed to upgrade database:', reopenRequest.error);
            reject(new Error('Failed to upgrade database'));
          };
        } else {
          resolve();
        }
      };

      request.onupgradeneeded = event => {
        const db = request.result;
        // Only create our store if this is a fresh database
        if (
          event.oldVersion === 0 &&
          !db.objectStoreNames.contains(this.storeName)
        ) {
          db.createObjectStore(this.storeName);
        }
      };
    });

    return this.dbInitPromise;
  }

  /**
   * Compute hash from a file or blob
   */
  async computeHash(blob: Blob): Promise<string> {
    const arrayBuffer = await blob.arrayBuffer();

    // Check if crypto.subtle is available (secure context)
    if (typeof crypto !== 'undefined' && crypto.subtle) {
      const hashBuffer = await crypto.subtle.digest('SHA-256', arrayBuffer);
      const hashArray = Array.from(new Uint8Array(hashBuffer));
      return hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
    } else {
      // Fallback for non-secure contexts
      console.warn('Web Crypto API not available, using fallback hash method');
      return this.fallbackHash(arrayBuffer);
    }
  }

  /**
   * Fallback hash function for environments where crypto.subtle is not available
   * This is a simple implementation and not as secure as the Web Crypto API
   */
  private fallbackHash(buffer: ArrayBuffer): string {
    const data = new Uint8Array(buffer);
    let hash = 0;

    for (let i = 0; i < data.length; i++) {
      // Simple hash algorithm
      hash = (hash << 5) - hash + data[i];
      hash |= 0; // Convert to 32-bit integer
    }

    // Convert to hex string and add timestamp to reduce collision chance
    const timestamp = Date.now().toString(16);
    const hashHex = (hash >>> 0).toString(16).padStart(8, '0');
    return hashHex + timestamp;
  }

  /**
   * Store a file and get its metadata
   */
  async addFile(file: File): Promise<FileMetadata> {
    await this.init();

    // Compute content hash
    const hash = await this.computeHash(file);

    // Store the blob using the hash as key
    await this.storeBlob(hash, file);

    // Return file metadata
    return {
      hash,
      name: file.name,
      type: file.type,
      size: file.size,
      lastModified: file.lastModified,
    };
  }

  /**
   * Store a blob in IndexedDB
   */
  async storeBlob(hash: string, blob: Blob): Promise<void> {
    if (!this.db) {
      throw new Error('Database not initialized');
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db!.transaction(this.storeName, 'readwrite');
      const store = transaction.objectStore(this.storeName);

      const request = store.put(blob, hash);

      request.onsuccess = () => resolve();
      request.onerror = () => reject(new Error('Failed to store blob'));

      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(new Error('Transaction failed'));
    });
  }

  /**
   * Get a blob from IndexedDB by its hash
   */
  async getBlob(hash: string): Promise<Blob | null> {
    await this.init();

    if (!this.db) {
      throw new Error('Database not initialized');
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db!.transaction(this.storeName, 'readonly');
      const store = transaction.objectStore(this.storeName);

      const request = store.get(hash);

      request.onsuccess = () => resolve(request.result || null);
      request.onerror = () => reject(new Error('Failed to retrieve blob'));
    });
  }

  /**
   * Check if a blob exists by its hash
   */
  async hasBlob(hash: string): Promise<boolean> {
    const blob = await this.getBlob(hash);
    return blob !== null;
  }

  /**
   * Delete a blob by its hash
   */
  async deleteBlob(hash: string): Promise<void> {
    await this.init();

    if (!this.db) {
      throw new Error('Database not initialized');
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db!.transaction(this.storeName, 'readwrite');
      const store = transaction.objectStore(this.storeName);

      const request = store.delete(hash);

      request.onsuccess = () => resolve();
      request.onerror = () => reject(new Error('Failed to delete blob'));

      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(new Error('Transaction failed'));
    });
  }

  /**
   * Create a URL for a blob
   */
  async createBlobUrl(hash: string): Promise<string> {
    const blob = await this.getBlob(hash);
    if (!blob) {
      throw new Error(`Blob with hash ${hash} not found`);
    }
    return URL.createObjectURL(blob);
  }

  /**
   * Revoke a blob URL
   */
  revokeBlobUrl(url: string): void {
    URL.revokeObjectURL(url);
  }

  /**
   * Export a file to the user's filesystem
   */
  async exportFile(metadata: FileMetadata): Promise<void> {
    const blob = await this.getBlob(metadata.hash);
    if (!blob) {
      throw new Error(`Blob with hash ${metadata.hash} not found`);
    }

    // Check if File System Access API is available
    if ('showSaveFilePicker' in window) {
      try {
        // Type assertion for the File System Access API
        const showSaveFilePicker = (window as any)
          .showSaveFilePicker as (options: {
          suggestedName?: string;
          types?: Array<{
            description: string;
            accept: Record<string, string[]>;
          }>;
        }) => Promise<FileSystemFileHandle>;

        const fileHandle = await showSaveFilePicker({
          suggestedName: metadata.name,
          types: [
            {
              description: 'File',
              accept: {[metadata.type]: [`.${metadata.name.split('.').pop()}`]},
            },
          ],
        });

        const writable = await fileHandle.createWritable();
        await writable.write(blob);
        await writable.close();
      } catch (err: unknown) {
        // Properly type the error
        const error = err as {name?: string; message?: string};
        if (error.name !== 'AbortError') {
          throw new Error(
            `Failed to export file: ${error.message || 'Unknown error'}`,
          );
        }
      }
    } else {
      // Fallback for browsers without File System Access API
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = metadata.name;
      a.click();
      URL.revokeObjectURL(url);
    }
  }

  /**
   * Import a file from the user's filesystem
   */
  async importFile(): Promise<FileMetadata | null> {
    // Check if File System Access API is available
    if ('showOpenFilePicker' in window) {
      try {
        // Type assertion for the File System Access API
        const showOpenFilePicker = (window as any)
          .showOpenFilePicker as () => Promise<FileSystemFileHandle[]>;

        const [fileHandle] = await showOpenFilePicker();
        const file = await fileHandle.getFile();
        return this.addFile(file);
      } catch (err: unknown) {
        // Properly type the error
        const error = err as {name?: string; message?: string};
        if (error.name !== 'AbortError') {
          throw new Error(
            `Failed to import file: ${error.message || 'Unknown error'}`,
          );
        }
        return null;
      }
    } else {
      // Fallback for browsers without File System Access API
      return new Promise((resolve, reject) => {
        const input = document.createElement('input');
        input.type = 'file';

        input.onchange = async () => {
          const file = input.files?.[0];
          if (file) {
            try {
              const metadata = await this.addFile(file);
              resolve(metadata);
            } catch (err: unknown) {
              reject(err);
            }
          } else {
            resolve(null);
          }
        };

        input.click();
      });
    }
  }

  /**
   * Close the database connection
   */
  close(): void {
    if (this.db) {
      this.db.close();
      this.db = null;
    }
  }
}

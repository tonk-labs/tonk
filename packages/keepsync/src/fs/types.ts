export interface FileMetadata {
  hash: string;
  name: string;
  type: string;
  size: number;
  lastModified: number;
  path?: string; // Optional path for organizing files
}

export interface FileManagerOptions {
  /**
   * Name of the IndexedDB database to use
   * @default 'sync-engine-store' (same as the Storage class)
   */
  dbName?: string;

  /**
   * Name of the object store for file blobs
   * @default 'file-blobs'
   */
  storeName?: string;
}

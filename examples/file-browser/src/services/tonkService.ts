import { DirectoryEntry, NodeMetadata, TonkCore } from '@tonk/core';

const URL = 'localhost:8081';
const manifestUrl = `http://${URL}/.manifest.tonk`;
const wsUrl = `ws://${URL}`;

const response = await fetch(manifestUrl);
const manifest = await response.arrayBuffer();
const manifestBytes = new Uint8Array(manifest);

const tonk = await TonkCore.fromBytes(manifestBytes, {
  storage: { type: 'indexeddb' },
});

tonk.connectWebsocket(wsUrl);

/**
 * Service for interacting with tonk file system
 */
export const TonkService = {
  /**
   * Read a document from tonk
   * @param path Path to the document
   * @returns Document content or undefined if not found
   */
  async readDocument<T>(path: string): Promise<T | undefined> {
    try {
      return (await tonk.readFile(path)) as T;
    } catch (error) {
      console.error('Error reading document:', error);
      throw error;
    }
  },

  /**
   * Write a document to tonk
   * @param path Path to the document
   * @param content Content to write
   */
  async writeDocument<T>(path: string, content: T): Promise<void> {
    try {
      await tonk.createFile(path, content);
    } catch (error) {
      console.error('Error writing document:', error);
      throw error;
    }
  },

  /**
   * List contents of a directory with metadata
   * @param path Path to the directory
   * @returns Array of tuples [DirectoryEntry, NodeMetadata] or undefined if not found
   */
  async listDirectory(
    path: string
  ): Promise<[DirectoryEntry, NodeMetadata][] | undefined> {
    try {
      const entries = await tonk.listDirectory(path);
      if (!entries) return undefined;

      const entriesWithMetadata = await Promise.all(
        entries.map(async (entry): Promise<[DirectoryEntry, NodeMetadata]> => {
          const entryPath = path.endsWith('/')
            ? `${path}${entry.name}`
            : `${path}/${entry.name}`;
          const metadata = await tonk.getMetadata(entryPath);
          return [entry, metadata];
        })
      );

      return entriesWithMetadata;
    } catch (error) {
      console.error('Error listing directory:', error);
      throw error;
    }
  },

  /**
   * Create a directory
   * @param path Path to create the directory
   * @returns Created directory node or undefined if creation failed
   */
  async createDirectory(path: string): Promise<void> {
    try {
      return await tonk.createDirectory(path);
    } catch (error) {
      console.error('Error creating directory:', error);
      throw error;
    }
  },

  /**
   * Remove a document or directory
   * @param path Path to the document or directory
   * @returns True if removal was successful
   */
  async removeItem(path: string): Promise<boolean> {
    try {
      return await tonk.deleteFile(path);
    } catch (error) {
      console.error('Error removing item:', error);
      throw error;
    }
  },

  /**
   * Get file name from path
   * @param path Full path
   * @returns File name
   */
  getFileName(path: string): string {
    const parts = path.split('/');
    return parts[parts.length - 1];
  },

  /**
   * Get parent directory path
   * @param path Current path
   * @returns Parent directory path
   */
  getParentPath(path: string): string {
    const parts = path.split('/');
    parts.pop();
    return parts.join('/') || '/';
  },

  /**
   * Join path segments
   * @param base Base path
   * @param segment Path segment to add
   * @returns Joined path
   */
  joinPath(base: string, segment: string): string {
    if (base === '/') {
      return `/${segment}`;
    }
    return `${base}/${segment}`;
  },

  /**
   * Format timestamp to readable date
   * @param timestamp Timestamp in milliseconds
   * @returns Formatted date string
   */
  formatDate(timestamp: number): string {
    return new Date(timestamp).toLocaleString();
  },
};

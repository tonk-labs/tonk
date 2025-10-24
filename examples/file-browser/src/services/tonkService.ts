import { RefNode, TonkCore, JsonValue } from '@tonk/core';

let tonkInstance: TonkCore | null = null;
let isInitialized = false;

/**
 * Service for interacting with tonk file system
 */
export const TonkService = {
  /**
   * Initialize TonkCore with a relay server
   * @param relayUrl URL of the relay server (e.g., http://localhost:8081)
   */
  async initialize(relayUrl: string): Promise<void> {
    const manifestUrl = `${relayUrl}/.manifest.tonk`;
    const wsUrl = relayUrl.replace(/^http/, 'ws');

    try {
      const response = await fetch(manifestUrl);
      if (!response.ok) {
        throw new Error(`Failed to fetch manifest: ${response.statusText}`);
      }

      const manifest = await response.arrayBuffer();
      const manifestBytes = new Uint8Array(manifest);

      tonkInstance = await TonkCore.fromBytes(manifestBytes);

      await tonkInstance.connectWebsocket(wsUrl);
      isInitialized = true;

      console.log('TonkCore initialized successfully with relay:', relayUrl);
    } catch (error) {
      isInitialized = false;
      tonkInstance = null;
      console.error('Failed to initialize TonkCore:', error);
      throw error;
    }
  },

  /**
   * Check if TonkCore is initialized
   */
  isReady(): boolean {
    return isInitialized && tonkInstance !== null;
  },
  /**
   * Read a document from tonk
   * @param path Path to the document
   * @returns Document content or undefined if not found
   */
  async readDocument<T>(path: string): Promise<T | undefined> {
    if (!tonkInstance) {
      throw new Error('TonkCore not initialized. Call initialize() first.');
    }
    try {
      return (await tonkInstance.readFile(path)) as T;
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
  async writeDocument(path: string, content: JsonValue): Promise<void> {
    if (!tonkInstance) {
      throw new Error('TonkCore not initialized. Call initialize() first.');
    }
    try {
      await tonkInstance.createFile(path, content);
    } catch (error) {
      console.error('Error writing document:', error);
      throw error;
    }
  },

  /**
   * List contents of a directory
   * @param path Path to the directory
   * @returns Array of RefNode entries or undefined if not found
   */
  async listDirectory(path: string): Promise<RefNode[] | undefined> {
    if (!tonkInstance) {
      throw new Error('TonkCore not initialized. Call initialize() first.');
    }
    try {
      const entries = await tonkInstance.listDirectory(path);
      if (!entries) return undefined;

      return entries;
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
    if (!tonkInstance) {
      throw new Error('TonkCore not initialized. Call initialize() first.');
    }
    try {
      return await tonkInstance.createDirectory(path);
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
    if (!tonkInstance) {
      throw new Error('TonkCore not initialized. Call initialize() first.');
    }
    try {
      return await tonkInstance.deleteFile(path);
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

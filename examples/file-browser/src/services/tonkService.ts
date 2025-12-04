import {
  RefNode,
  TonkCore,
  JsonValue,
  DirectoryNode,
  DocumentWatcher,
  Bundle,
  Manifest,
} from '@tonk/core';

let tonkInstance: TonkCore | null = null;
let isInitialized = false;
let currentWsUrl: string | null = null;

/**
 * Filter network URIs to only HTTP(S) relay URLs
 */
function getHttpRelays(networkUris: string[]): string[] {
  return networkUris.filter(
    uri => uri.startsWith('http://') || uri.startsWith('https://')
  );
}

/**
 * Service for interacting with tonk file system
 */
export const TonkService = {
  /**
   * Initialize TonkCore from bundle bytes
   * @param bundleBytes The bundle data as Uint8Array
   * @param wsUrlOverride Optional WebSocket URL to override manifest's networkUris
   * @returns The parsed manifest
   */
  async initializeFromBundle(
    bundleBytes: Uint8Array,
    wsUrlOverride?: string
  ): Promise<{ manifest: Manifest; connectedRelay: string | null }> {
    try {
      // Parse bundle to get manifest
      const bundle = await Bundle.fromBytes(bundleBytes);
      const manifest = await bundle.getManifest();
      bundle.free();

      // Create TonkCore with IndexedDB storage
      tonkInstance = await TonkCore.fromBytes(bundleBytes, {
        storage: { type: 'indexeddb' },
      });

      // Determine relay URL: override > first HTTP relay from manifest
      const httpRelays = getHttpRelays(manifest.networkUris || []);
      const relayUrl = wsUrlOverride || httpRelays[0];
      let connectedRelay: string | null = null;

      // Connect to relay if available
      if (relayUrl) {
        const wsUrl = relayUrl.replace(/^http/, 'ws');
        await tonkInstance.connectWebsocket(wsUrl);
        currentWsUrl = wsUrl;
        connectedRelay = relayUrl;
      }

      isInitialized = true;
      return { manifest, connectedRelay };
    } catch (error) {
      isInitialized = false;
      tonkInstance = null;
      currentWsUrl = null;
      console.error('Failed to initialize TonkCore:', error);
      throw error;
    }
  },

  /**
   * Connect to a relay WebSocket
   * @param relayUrl HTTP(S) URL of the relay server
   */
  async connectRelay(relayUrl: string): Promise<void> {
    if (!tonkInstance) {
      throw new Error('TonkCore not initialized. Load a bundle first.');
    }
    const wsUrl = relayUrl.replace(/^http/, 'ws');
    await tonkInstance.connectWebsocket(wsUrl);
    currentWsUrl = wsUrl;
  },

  /**
   * Get the currently connected relay URL
   */
  getConnectedRelay(): string | null {
    return currentWsUrl ? currentWsUrl.replace(/^ws/, 'http') : null;
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

  /**
   * Watch a directory for changes
   * @param path Path to the directory
   * @param callback Callback to run on change events
   * @returns A DocumentWatcher for the specified path
   */
  async watchDirectory(
    path: string,
    callback: (result: DirectoryNode) => void
  ): Promise<DocumentWatcher> {
    if (!tonkInstance) {
      throw new Error('TonkCore not initialized. Call initialize() first.');
    }
    try {
      return await tonkInstance.watchDirectory(path, callback);
    } catch (error) {
      console.error('Error watching directory:', error);
      throw error;
    }
  },
};

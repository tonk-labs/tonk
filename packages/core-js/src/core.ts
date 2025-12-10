import type { WasmBundle, WasmTonkCore } from './tonk_core.js';

/**
 * Entry in a directory listing
 */
export interface RefNode {
  /** Name of the file or directory */
  name: string;
  /** Type of the entry */
  type: 'directory' | 'document';
  //the UUID of the automerge URL
  pointer: string;
  timestamps: DocumentTimestamps;
}

export interface DirectoryNode {
  /** Name of the file or directory */
  name: string;
  /** Type of the entry */
  type: 'directory';
  //the UUID of the automerge URL
  pointer: string;
  timestamps: DocumentTimestamps;
  // if it's a directory this will be populated
  children?: RefNode[];
}

/**
 * Entry returned by bundle prefix queries
 */
export interface BundleEntry {
  /** Key of the entry */
  key: string;
  /** Binary data stored at this key */
  value: Uint8Array;
}

export interface DocumentWatcher {
  /**
   * Gets the document ID being watched
   * @returns The document ID as a string
   */
  documentId(): string;

  /**
   * Stops the document watcher and aborts any ongoing watch operations
   * @returns Promise that resolves when the watcher is stopped
   */
  stop(): Promise<void>;
}

export interface DocumentTimestamps {
  created: number;
  modified: number;
}

/**
 * Type for structured JSON that must be an object or array at the top level.
 * Covers the full JSON specification but excludes primitive values at the root.
 */
export type JsonValue =
  | { [key: string]: JsonValue | null | boolean | number | string }
  | (JsonValue | null | boolean | number | string)[];

export interface DocumentData {
  content: JsonValue;
  name: string;
  timestamps: DocumentTimestamps;
  type: 'document' | 'directory';
  bytes?: string; // Base64-encoded binary data when file was created with bytes
}

/**
 * Tonk version
 */
export interface Version {
  major: number;
  minor: number;
}

/**
 * Bundle manifest
 */
export interface Manifest {
  /** Manifest version */
  manifestVersion: number;
  /** Tonk version */
  version: Version;
  rootId: string;
  entrypoints: string[];
  networkUris: string[];
  xNotes?: string;
  xVendor?: Object;
}

/**
 * Storage configuration options
 */
export interface StorageConfig {
  /** Storage type: 'memory' for in-memory storage, 'indexeddb' for IndexedDB storage */
  type: 'memory' | 'indexeddb';
}

/**
 * Configuration options for Tonk initialization
 */
export interface TonkConfig {
  /** Custom path to the WASM module */
  wasmPath?: string;
  /** Storage configuration */
  storage?: StorageConfig;
  /** Peer ID to use (auto-generated if not provided) */
  peerId?: string;
}

/**
 * Configuration for bundle export
 */
export interface BundleConfig {
  /** Entry points for the bundle (e.g., main application files) */
  entrypoints?: string[];
  /** Network URIs that the bundle may need to access */
  networkUris?: string[];
  /** Optional notes about the bundle */
  notes?: string;
  /** Custom vendor-specific metadata */
  vendorMetadata?: any;
}

/**
 * Base error class for all Tonk-related errors
 */
export class TonkError extends Error {
  constructor(
    message: string,
    public code?: string
  ) {
    super(message);
    this.name = 'TonkError';
  }
}

/**
 * Error thrown when connection operations fail
 */
export class ConnectionError extends TonkError {
  constructor(message: string) {
    super(message, 'CONNECTION_ERROR');
    this.name = 'ConnectionError';
  }
}

/**
 * Error thrown when file system operations fail
 */
export class FileSystemError extends TonkError {
  constructor(message: string) {
    super(message, 'FILESYSTEM_ERROR');
    this.name = 'FileSystemError';
  }
}

/**
 * Error thrown when bundle operations fail
 */
export class BundleError extends TonkError {
  constructor(message: string) {
    super(message, 'BUNDLE_ERROR');
    this.name = 'BundleError';
  }
}

// TODO: update docs for bundles
/**
 * Bundle for storing and retreiving binary data in a ZIP-like format.
 *
 * Bundles provide efficent storage and retrieval of key-value pairs
 * with support for prefix queries and serialization.
 *
 * @example
 * ```typescript
 * // Create a new bundle
 * const bundle = Bundle.create();
 *
 * // Store some data
 * const encoder = new TextEncoder();
 * await bundle.put('config.json', encoder.encode('{"version": 1}'));
 *
 * // Retrieve data
 * const data = await bundle.get('config.json');
 * if (data) {
 *   const decoder = new TextDecoder();
 *   console.log(decoder.decode(data));
 * }
 *
 * // Export bundle
 * const bytes = await bundle.toBytes();
 * ```
 */
export class Bundle {
  #wasm: WasmBundle;

  /** @internal */
  constructor(wasm: WasmBundle) {
    this.#wasm = wasm;
  }

  /**
   * Create a bundle from existing data.
   *
   * @param data - Binary data representing a serialized bundle
   * @param wasmModule - WASM module functions (for lazy loading)
   * @returns A new Bundle instance
   * @throws {BundleError} If the data is invalid, corrupted, or WASM not initialized
   */
  static async fromBytes(data: Uint8Array, wasmModule?: any): Promise<Bundle> {
    try {
      const { create_bundle_from_bytes } =
        wasmModule || (await import('./tonk_core.js'));
      return new Bundle(create_bundle_from_bytes(data));
    } catch (error) {
      throw new BundleError(`Failed to create bundle from bytes: ${error}`);
    }
  }

  /**
   * Retrieve the root ID from the bundle
   *
   * @returns The root ID
   * @throws {BundleError} If the operation fails
   */
  async getRootId(): Promise<string> {
    try {
      const rootId = await this.#wasm.getRootId();
      return rootId;
    } catch (error) {
      throw new BundleError(`Failed to get root ID: ${error}`);
    }
  }

  /**
   * Retrieve a value from the bundle.
   *
   * @param key - The key to look up
   * @returns The stored data, or null if the key doesn't exist
   * @throws {BundleError} If the operation fails
   */
  async get(key: string): Promise<Uint8Array | null> {
    try {
      const result = await this.#wasm.get(key);
      return result === null ? null : result;
    } catch (error) {
      throw new BundleError(`Failed to get key ${key}: ${error}`);
    }
  }

  /**
   * Get all entries with keys that start with the given prefix.
   *
   * @param prefix - The prefix to search for
   * @returns Array of matching entrys
   * @throws {BundleError} If the operation fails
   *
   * @example
   * ```typescript
   * // Get all config files
   * const configs = await bundle.getPrefix('config/');
   * for (const entry of configs) {
   *   console.log(`${entry.key}: ${entry.value.byteLength} bytes`);
   * }
   * ```
   */
  async getPrefix(prefix: string): Promise<BundleEntry[]> {
    try {
      const results = await this.#wasm.getPrefix(prefix);
      return results.map((entry: any) => ({
        key: entry.key,
        value: entry.value,
      }));
    } catch (error) {
      throw new BundleError(`Failed to get prefix ${prefix}: ${error}`);
    }
  }

  /**
   * List all keys in the bundle.
   *
   * @returns Array of all keys
   * @throws {BundleError} If the operation fails
   */
  async listKeys(): Promise<string[]> {
    try {
      return await this.#wasm.listKeys();
    } catch (error) {
      throw new BundleError(`Failed to list keys: ${error}`);
    }
  }

  /**
   * Retrieve the bundle manifest
   *
   * @returns Manifest as JSON
   * @throws {BundleError} If the operation fails
   */
  async getManifest(): Promise<Manifest> {
    try {
      return await this.#wasm.getManifest();
    } catch (error) {
      throw new BundleError(`Failed to retrieve manifest: ${error}`);
    }
  }

  /**
   * Update the bundle manifest with new configuration
   *
   * @param config Bundle configuration to apply to the manifest
   * @throws {BundleError} If the operation fails
   */
  async setManifest(config: BundleConfig): Promise<void> {
    try {
      await (this.#wasm as any).setManifest(config);
    } catch (error) {
      throw new BundleError(`Failed to set manifest: ${error}`);
    }
  }

  /**
   * Serialize the bundle to binary data.
   *
   * @returns The serialized bundle data
   * @throws {BundleError} If serialization fails
   */
  async toBytes(): Promise<Uint8Array> {
    try {
      return await this.#wasm.toBytes();
    } catch (error) {
      throw new BundleError(`Failed to serialize bundle: ${error}`);
    }
  }

  /**
   * Free the WASM memory associated with this bundle.
   * Call this when you're done with the bundle to prevent memory leaks.
   */
  free(): void {
    this.#wasm.free();
  }
}

/**
 * Main syncronization engine for Tonk.
 *
 * The TonkCore manages peer-to-peer synchronization, WebSocket connections,
 * and access to the virtual file system.
 *
 * @example
 * ```typescript
 * // Create a Tonk Core with auto-generated peer ID
 * const tonk = await TonkCore.create();
 *
 * // Or create with a specific peer ID
 * const tonk = await TonkCore.createWithPeerId('my-peer-id');
 *
 * // Connect to a sync server
 * await tonk.connectWebsocket('ws://localhost:8081');
 * ```
 */
export class TonkCore {
  #wasm: WasmTonkCore;

  /** @internal */
  private constructor(wasm: WasmTonkCore) {
    this.#wasm = wasm;
  }

  /**
   * Create a new Tonk Core with an auto-generated peer ID.
   *
   * @param config - Configuration options
   * @param wasmModule - WASM module functions (for lazy loading)
   * @returns A new TonkCore instance
   * @throws {Error} If Tonk creation fails or WASM not initialized
   *
   * @example
   * ```typescript
   * // Default in-memory storage
   * const tonk = await TonkCore.create();
   *
   * // With IndexedDB storage
   * const tonk = await TonkCore.create({ storage: { type: 'indexeddb' } });
   *
   * // With custom peer ID
   * const tonk = await TonkCore.create({
   *   peerId: 'my-custom-peer-id',
   *   storage: { type: 'indexeddb' }
   * });
   * ```
   */
  static async create(
    config?: TonkConfig,
    wasmModule?: any
  ): Promise<TonkCore> {
    const module = wasmModule || (await import('./tonk_core.js'));

    if (config?.peerId && config?.storage) {
      const { create_tonk_with_config } = module;
      const wasm = await create_tonk_with_config(
        config.peerId,
        config.storage.type === 'indexeddb'
      );
      return new TonkCore(wasm);
    } else if (config?.peerId) {
      const { create_tonk_with_peer_id } = module;
      const wasm = await create_tonk_with_peer_id(config.peerId);
      return new TonkCore(wasm);
    } else if (config?.storage) {
      const { create_tonk_with_storage } = module;
      const wasm = await create_tonk_with_storage(
        config.storage.type === 'indexeddb'
      );
      return new TonkCore(wasm);
    } else {
      const { create_tonk } = module;
      const wasm = await create_tonk();
      return new TonkCore(wasm);
    }
  }

  /**
   * Create a new Tonk Core with a specific peer ID.
   *
   * @param peerId - The peer ID to use
   * @param wasmModule - WASM module functions (for lazy loading)
   * @returns A new TonkCore instance
   * @throws {Error} If Tonk creation fails, peer ID is invalid, or WASM not initialized
   */
  static async createWithPeerId(
    peerId: string,
    wasmModule?: any
  ): Promise<TonkCore> {
    const { create_tonk_with_peer_id } =
      wasmModule || (await import('./tonk_core.js'));
    const wasm = await create_tonk_with_peer_id(peerId);
    return new TonkCore(wasm);
  }

  /**
   * Create a new Tonk Core from an existing bundle
   *
   * @param bundle - The Bundle instance from which to load
   * @param config - Configuration options
   * @param wasmModule - WASM module functions (for lazy loading)
   * @returns A new TonkCore instance
   * @throws {Error} If Tonk creation fails, bundle is invalid, or WASM not initialized
   *
   * @example
   * ```typescript
   * // Load with in-memory storage (default)
   * const tonk = await TonkCore.fromBundle(bundle);
   *
   * // Load with IndexedDB storage
   * const tonk = await TonkCore.fromBundle(bundle, {
   *   storage: { type: 'indexeddb' }
   * });
   * ```
   */
  static async fromBundle(
    bundle: Bundle,
    config?: TonkConfig,
    wasmModule?: any
  ): Promise<TonkCore> {
    const module = wasmModule || (await import('./tonk_core.js'));

    if (config?.storage) {
      const { create_tonk_from_bundle_with_storage } = module;
      const wasm = await create_tonk_from_bundle_with_storage(
        bundle,
        config.storage.type === 'indexeddb'
      );
      return new TonkCore(wasm);
    } else {
      const { create_tonk_from_bundle } = module;
      const wasm = await create_tonk_from_bundle(bundle);
      return new TonkCore(wasm);
    }
  }

  /**
   * Create a new Tonk Core from bundle data
   *
   * @param data - The Bundle data from which to load
   * @param config - Configuration options
   * @param wasmModule - WASM module functions (for lazy loading)
   * @returns A new TonkCore instance
   * @throws {Error} If Tonk creation fails, bundle is invalid, or WASM not initialized
   *
   * @example
   * ```typescript
   * // Load with in-memory storage (default)
   * const tonk = await TonkCore.fromBytes(bundleData);
   *
   * // Load with IndexedDB storage
   * const tonk = await TonkCore.fromBytes(bundleData, {
   *   storage: { type: 'indexeddb' }
   * });
   * ```
   */
  static async fromBytes(
    data: Uint8Array,
    config?: TonkConfig,
    wasmModule?: any
  ): Promise<TonkCore> {
    const module = wasmModule || (await import('./tonk_core.js'));

    if (config?.storage) {
      const { create_tonk_from_bytes_with_storage } = module;
      const wasm = await create_tonk_from_bytes_with_storage(
        data,
        config.storage.type === 'indexeddb'
      );
      return new TonkCore(wasm);
    } else {
      const { create_tonk_from_bytes } = module;
      const wasm = await create_tonk_from_bytes(data);
      return new TonkCore(wasm);
    }
  }

  /**
   * Get the peer ID of this Tonk Core
   *
   * @returns The peer ID as a string
   */
  getPeerId(): Promise<string> {
    return this.#wasm.getPeerId();
  }

  /**
   * Connect to a WebSocket server for real-time syncronization.
   *
   * @param url - WebSocket URL to connect to
   * @throws {ConnectionError} If connection fails
   *
   * @example
   * ```typescript
   * await tonk.connectWebsocket('ws://sync.example.com:8081');
   * ```
   */
  async connectWebsocket(url: string): Promise<void> {
    try {
      await this.#wasm.connectWebsocket(url);
    } catch (error) {
      throw new ConnectionError(`Failed to connect to ${url}: ${error}`);
    }
  }

  /**
   * Check if currently connected to WebSocket.
   *
   * @returns True if connected, false otherwise
   */
  async isConnected(): Promise<boolean> {
    try {
      const result = await this.#wasm.isConnected();
      return result;
    } catch (error) {
      console.error('🔌 [CORE-JS] isConnected() error:', error);
      return false;
    }
  }

  /**
   * Get the current connection state.
   *
   * @returns Connection state as a string: "disconnected", "connecting", "connected", or "failed:{message}"
   */
  async getConnectionState(): Promise<string> {
    try {
      const result = await this.#wasm.getConnectionState();
      return result;
    } catch (error) {
      console.error('📡 [CORE-JS] getConnectionState() error:', error);
      return 'failed:' + String(error);
    }
  }

  /**
   * Serialize the Tonk Core /app data to a fresh binary with a new root.
   *
   * @param config Optional configuration for bundle export
   * @returns The serialized bundle data
   * @throws {TonkError} If serialization fails
   */
  async forkToBytes(config?: BundleConfig): Promise<Uint8Array> {
    try {
      return await this.#wasm.forkToBytes(config);
    } catch (error) {
      throw new TonkError(`Failed to serialize to bundle data: ${error}`);
    }
  }

  /**
   * Serialize the Tonk Core to bundle binary data.
   *
   * @param config Optional configuration for bundle export
   * @returns The serialized bundle data
   * @throws {TonkError} If serialization fails
   */
  async toBytes(config?: BundleConfig): Promise<Uint8Array> {
    try {
      return await this.#wasm.toBytes(config);
    } catch (error) {
      throw new TonkError(`Failed to serialize to bundle data: ${error}`);
    }
  }

  /**
   * Create a new file with the given content.
   *
   * @param path - Absolute path where the file should be created
   * @param content - Content to write to the file (any JSON-serializable value)
   * @throws {FileSystemError} If the file already exists or path is invalid
   *
   * @example
   * ```typescript
   *
   * // Create a JSON file with an object
   * await createFile('/config.json', { theme: 'dark', fontSize: 14 });
   *
   * // Create a file with an array
   * await createFile('/data.json', [1, 2, 3, 4, 5]);
   * ```
   */
  async createFile(path: string, content: JsonValue): Promise<void> {
    try {
      await this.#wasm.createFile(path, content);
    } catch (error) {
      throw new FileSystemError(`Failed to create file at ${path}: ${error}`);
    }
  }

  /**
   * Create a new file with the given content and byte array.
   *
   * @param path - Absolute path where the file should be created
   * @param content - Content to write to the file (any JSON-serializable value)
   * @param bytes - The binary data you want to store (should be Uint8Array)
   * @throws {FileSystemError} If the file already exists or path is invalid
   *
   * @example
   * ```typescript
   * // Create a media file with JSON metadata
   * await createFile('/tree.png', { mime: 'image/png', alt: 'picture of a tree' }, encodedImageData);
   * ```
   */
  async createFileWithBytes(
    path: string,
    content: JsonValue,
    bytes: Uint8Array | string
  ): Promise<void> {
    try {
      const normalizedBytes: Uint8Array =
        typeof bytes === 'string' ? extractBytes(bytes) : bytes;

      await this.#wasm.createFileWithBytes(path, content, normalizedBytes);
    } catch (error) {
      throw new FileSystemError(`Failed to create file at ${path}: ${error}`);
    }
  }

  /**
   * Read the contents of a file.
   *
   * @param path - Absolute path to the file
   * @returns An object containing the file data
   * @throws {FileSystemError} If the file doesn't exist or can't be read
   *
   * @example
   * ```typescript
   * const doc = await readFile('/notes/todo');
   * console.log(doc.content);   // string
   * console.log(doc.name);      // string
   * console.log(doc.type);      // 'document' | 'directory'
   * console.log(doc.timestamps.created);   // number
   * console.log(doc.timestamps.modified);  // number
   * ```
   */

  async readFile(path: string): Promise<DocumentData> {
    try {
      const intermediary = await this.#wasm.readFile(path);
      if (intermediary === null) {
        throw new FileSystemError(`File not found: ${path}`);
      }

      // For some reason, we seem to get different return types in different environments
      let normalizedBytes;
      if (intermediary.bytes) {
        normalizedBytes = normalizeBytes(intermediary.bytes);
      }

      return {
        ...intermediary,
        content:
          typeof intermediary.content === 'string'
            ? JSON.parse(intermediary.content)
            : intermediary.content,
        bytes: normalizedBytes,
      };
    } catch (error) {
      if (error instanceof FileSystemError) throw error;
      throw new FileSystemError(`Failed to read file at ${path}: ${error}`);
    }
  }

  /**
   * Set an existing file with the given content.
   *
   * @param path - Absolute path of the file to set
   * @param content - Content to write to the file (any JSON-serializable value)
   * @returns true if the file was set, false if it didn't exist
   * @throws {FileSystemError} If the path is invalid
   *
   * @example
   * ```typescript
   * // Create a text file
   * await createFile('/hello.txt', 'Hello, World!');
   *
   * // Overwrite it with a string
   * await setFile('/hello.txt', 'See you later!');
   *
   * // Set with an object
   * await setFile('/config.json', { theme: 'light', fontSize: 16 });
   * ```
   */
  async setFile(path: string, content: JsonValue): Promise<boolean> {
    try {
      return await this.#wasm.setFile(path, content);
    } catch (error) {
      throw new FileSystemError(`Failed to set file at ${path}: ${error}`);
    }
  }

  /**
   * Set an existing file with the given content and bytes.
   *
   * @param path - Absolute path of the file to set
   * @param content - Content to write to the file (any JSON-serializable value)
   * @param bytes - Binary data to store
   * @returns true if the file was set, false if it didn't exist
   * @throws {FileSystemError} If the path is invalid
   *
   * @example
   * ```typescript
   * // Create a file with bytes
   * await createFileWithBytes('/image.json', { type: 'png' }, imageData);
   *
   * // Overwrite it
   * await setFileWithBytes('/image.json', { type: 'jpeg' }, newImageData);
   * ```
   */
  async setFileWithBytes(
    path: string,
    content: JsonValue,
    bytes: Uint8Array | string
  ): Promise<boolean> {
    try {
      const normalizedBytes: Uint8Array =
        typeof bytes === 'string' ? extractBytes(bytes) : bytes;
      return await this.#wasm.setFileWithBytes(path, content, normalizedBytes);
    } catch (error) {
      throw new FileSystemError(`Failed to set file at ${path}: ${error}`);
    }
  }

  /**
   * Update an existing file with intelligent diffing.
   * Compares the new content against existing content and applies minimal patches.
   *
   * @param path - Absolute path of the file to update
   * @param content - New content for the file (any JSON-serializable value)
   * @returns true if changes were made, false if content was unchanged or file didn't exist
   * @throws {FileSystemError} If the path is invalid
   *
   * @example
   * ```typescript
   * // Only the 'theme' field will be patched, other fields remain untouched
   * await updateFile('/config.json', { theme: 'dark', fontSize: 14 });
   *
   * // Adding new keys and removing missing ones
   * await updateFile('/data.json', { newKey: 'value' }); // removes old keys not in new content
   * ```
   */
  async updateFile(path: string, content: JsonValue): Promise<boolean> {
    try {
      return await (this.#wasm as any).updateFile(path, content);
    } catch (error) {
      throw new FileSystemError(`Failed to update file at ${path}: ${error}`);
    }
  }

  /**
   * Patch a file at a specific JSON path.
   *
   * @param path - Absolute path of the file to patch
   * @param jsonPath - Array of keys to the field to update, e.g. ['position', 'x']
   * @param value - New value for the field (any JSON-serializable value)
   * @returns true if the file was patched, false if it didn't exist
   * @throws {FileSystemError} If the path is invalid or patch fails
   *
   * @example
   * ```typescript
   * // Update just the x coordinate - minimal storage overhead
   * await patchFile('/layout/file1.json', ['x'], 150);
   *
   * // Update nested field
   * await patchFile('/config.json', ['settings', 'theme'], 'dark');
   *
   * // Update array element (using numeric string key)
   * await patchFile('/data.json', ['items', '0', 'name'], 'New Name');
   * ```
   */
  async patchFile(
    path: string,
    jsonPath: string[],
    value: JsonValue | string | number | boolean | null
  ): Promise<boolean> {
    try {
      return await (this.#wasm as any).patchFile(path, jsonPath, value);
    } catch (error) {
      throw new FileSystemError(`Failed to patch file at ${path}: ${error}`);
    }
  }

  /**
   * Splice text at a specific JSON path within a document.
   *
   * @param path - Absolute path of the file to modify
   * @param jsonPath - Array of keys to the text field, e.g. ['content']
   * @param index - Position in the text to start modification
   * @param deleteCount - Number of characters to delete (0 to insert only)
   * @param insert - String to insert at the position (empty string to delete only)
   * @returns true if the file was modified, false if it didn't exist
   * @throws {FileSystemError} If the path is invalid or splice fails
   *
   * @example
   * ```typescript
   * // Insert " world" at position 5 in the "content" field
   * await spliceText('/notes.json', ['content'], 5, 0, ' world');
   *
   * // Delete 3 characters starting at position 10
   * await spliceText('/notes.json', ['content'], 10, 3, '');
   *
   * // Replace "old" with "new" at position 5 (delete 3, insert 3)
   * await spliceText('/notes.json', ['content'], 5, 3, 'new');
   * ```
   */
  async spliceText(
    path: string,
    jsonPath: string[],
    index: number,
    deleteCount: number,
    insert: string
  ): Promise<boolean> {
    try {
      return await (this.#wasm as any).spliceText(
        path,
        jsonPath,
        index,
        deleteCount,
        insert
      );
    } catch (error) {
      throw new FileSystemError(`Failed to splice text at ${path}: ${error}`);
    }
  }

  /**
   * Delete a file.
   *
   * @param path - Absolute path to the file
   * @returns true if the file was deleted, false if it didn't exist
   * @throws {FileSystemError} If the deletion fails
   */
  async deleteFile(path: string): Promise<boolean> {
    try {
      return await this.#wasm.deleteFile(path);
    } catch (error) {
      throw new FileSystemError(`Failed to delete file at ${path}: ${error}`);
    }
  }

  /**
   * Create a new directoy.
   *
   * @param path - Absolute path where the directory should be created
   * @throws {FileSystemError} If the directory already exists or path is invalid
   *
   * @example
   * ```typescript
   * await createDirectory('/projects/my-app');
   * ```
   */
  async createDirectory(path: string): Promise<void> {
    try {
      await this.#wasm.createDirectory(path);
    } catch (error) {
      throw new FileSystemError(
        `Failed to create directory at ${path}: ${error}`
      );
    }
  }

  /**
   * List the contents of a directory.
   *
   * @param path - Absolute path to the directory
   * @returns Array of directory entries
   * @throws {FileSystemError} If the directory doesn't exist or can't be read
   *
   * @example
   * ```typescript
   * const entries = await listDirectory('/projects');
   * for (const entry of entries) {
   *   console.log(`${entry.type}: ${entry.name} ${entry.timestamps} ${entry.pointer}`);
   * }
   * ```
   */
  async listDirectory(path: string): Promise<RefNode[]> {
    try {
      const entries = await this.#wasm.listDirectory(path);
      return entries.map((entry: any) => ({
        name: entry.name,
        type: entry.type as 'directory' | 'document',
        timestamps: entry.timestamps,
        pointer: entry.pointer,
      }));
    } catch (error) {
      throw new FileSystemError(
        `Failed to list directory at ${path}: ${error}`
      );
    }
  }

  /**
   * Check if a file or directory exists at the given path.
   *
   * @param path - Absolute path to check
   * @returns true if something exists at the path, false otherwise
   */
  async exists(path: string): Promise<boolean> {
    try {
      return await this.#wasm.exists(path);
    } catch (error) {
      throw new FileSystemError(
        `Failed to check existence of ${path}: ${error}`
      );
    }
  }

  /**
   * Rename a file or directory.
   *
   * @param oldPath - Absolute path of the file or directory to rename
   * @param newPath - Absolute path of the new name
   * @returns true if the rename was successful, false if the source doesn't exist
   * @throws {FileSystemError} If the rename fails or paths are invalid
   *
   * @example
   * ```typescript
   * // Rename a file
   * await rename('/old-name.txt', '/new-name.txt');
   *
   * // Rename a directory
   * await rename('/old-folder', '/new-folder');
   * ```
   */
  async rename(oldPath: string, newPath: string): Promise<boolean> {
    try {
      return await this.#wasm.rename(oldPath, newPath);
    } catch (error) {
      throw new FileSystemError(
        `Failed to rename ${oldPath} to ${newPath}: ${error}`
      );
    }
  }

  /**
   * Get metadata for a file or directory.
   *
   * @param path - Absolute path to the file or directory
   * @returns Metadata object containing file/directory information
   * @throws {FileSystemError} If the file or directory doesn't exist, or if the metadata can't be retrieved
   *
   * @example
   * ```typescript
   * try {
   *   const metadata = await getMetadata('/notes/todo');
   *   console.log(`Type: ${metadata.nodeType}`);
   *   console.log(`Created: ${metadata.createdAt}`);
   *   console.log(`Modified: ${metadata.modifiedAt}`);
   * } catch (error) {
   *   if (error instanceof FileSystemError) {
   *     console.error('File not found or access error:', error.message);
   *   }
   * }
   * ```
   */
  async getMetadata(path: string): Promise<RefNode> {
    try {
      const result = await this.#wasm.getMetadata(path);
      if (result === null) {
        throw new FileSystemError(`File or directory not found: ${path}`);
      }

      return result;
    } catch (error) {
      if (error instanceof FileSystemError) throw error;
      throw new FileSystemError(`Failed to get metadata for ${path}: ${error}`);
    }
  }

  /**
   * Watch a file for changes at the specified path
   *
   * @param path - Absolute path to the file
   * @param callback - Callback to run on change events
   * @returns A DocumentWatcher for the specified path
   *
   * @example
   * ```typescript
   * const watcher = await watchFile('/text.txt', docState => {
   *   console.log('Document changed:', docState);
   * });
   * ```
   */
  async watchFile(
    path: string,
    callback: (result: DocumentData) => void
  ): Promise<DocumentWatcher> {
    try {
      const result = await this.#wasm.watchDocument(path, (doc: any) => {
        let normalizedBytes;
        if (doc.bytes) {
          normalizedBytes = normalizeBytes(doc.bytes);
        }
        callback({
          ...doc,
          content:
            typeof doc.content === 'string'
              ? JSON.parse(doc.content)
              : doc.content,
          bytes: normalizedBytes,
        });
      });
      if (result === null) {
        throw new FileSystemError(`File not found: ${path}`);
      }

      return result;
    } catch (error) {
      if (error instanceof FileSystemError) throw error;
      throw new FileSystemError(
        `Failed to watch file at path ${path}: ${error}`
      );
    }
  }

  /**
   * Watch a directory will update only whenever it's direct descendents change.
   * You will need to keep track of the timestamps in the children entries to discover
   * which child has changed.
   *
   * @param path - Absolute path to the directory
   * @param callback - Callback to run on change events
   * @returns A DocumentWatcher for the specified path
   *
   * @example
   * ```typescript
   * const watcher = await watchDirectory('/documents', dirEntry => {
   *   console.log('Directory changed:', dirEntry);
   * });
   * ```
   */
  async watchDirectory(
    path: string,
    callback: (result: DirectoryNode) => void
  ): Promise<DocumentWatcher> {
    try {
      const result = await this.#wasm.watchDirectory(path, callback);
      if (result === null) {
        throw new FileSystemError(`Directory not found: ${path}`);
      }

      return result;
    } catch (error) {
      if (error instanceof FileSystemError) throw error;
      throw new FileSystemError(
        `Failed to watch directory at path ${path}: ${error}`
      );
    }
  }

  /**
   * Free the WASM memory associated with this Tonk Core.
   * Call this when you're done with the Tonk to prevent memory leaks.
   */
  free(): void {
    this.#wasm.free();
  }
}

/**
 * Factory functions that can work with either direct or lazy-loaded WASM modules
 */
export function createFactoryFunctions(wasmModule?: any) {
  return {
    /**
     * Create a new Tonk Core with an auto-generated peer ID.
     *
     * @returns A new TonkCore instance
     * @throws {Error} If Tonk creation fails
     */
    createTonk: () => TonkCore.create(wasmModule),

    /**
     * Create a new Tonk Core with a specific peer ID.
     *
     * @param peerId - The peer ID to use
     * @returns A new TonkCore instance
     * @throws {Error} If Tonk creation fails or peer ID is invalid
     */
    createTonkWithPeerId: (peerId: string) =>
      TonkCore.createWithPeerId(peerId, wasmModule),

    /**
     * Create a Tonk Core from an existing bundle
     * @param bundle - The Bundle instance from which to load
     * @returns A new TonkCore instance
     * @throws {Error} If Tonk creation fails or bundle is invalid
     */
    createTonkFromBundle: (bundle: Bundle) =>
      TonkCore.fromBundle(bundle, wasmModule),

    /**
     * Create a Tonk Core from bundle data
     * @param data - The Bundle data from which to load
     * @returns A new TonkCore instance
     * @throws {Error} If Tonk creation fails or bundle is invalid
     */
    createTonkFromBytes: (data: Uint8Array) =>
      TonkCore.fromBytes(data, wasmModule),

    /**
     * Create a bundle from existing data.
     *
     * @param data - Binary data representing a serialized bundle
     * @returns A new Bundle instance
     * @throws {BundleError} If the data is invalid or corrupted
     */
    createBundleFromBytes: (data: Uint8Array) =>
      Bundle.fromBytes(data, wasmModule),
  };
}

// Utility to turn base64 into Uint8Array
const extractBytes = (bytes: string) => {
  // Convert base64 string to Uint8Array
  const binaryString = atob(bytes);
  const barr = new Uint8Array(binaryString.length);
  for (let i = 0; i < binaryString.length; i++) {
    barr[i] = binaryString.charCodeAt(i);
  }
  return barr;
};

/**
 * Normalizes bytes from different formats to a base64 string
 * @param bytes - The bytes to normalize (can be string, array, or other format)
 * @returns A base64 encoded string
 * @throws FileSystemError if the bytes format is unrecognized
 */
const normalizeBytes = (bytes: any): string => {
  // Normalize bytes to always be base64 string
  if (typeof bytes === 'string') {
    // Already a base64 string
    return bytes;
  } else if (Array.isArray(bytes)) {
    // Convert array of char codes to base64 string
    // Process in chunks to avoid maximum call stack exceeded errors
    const chunkSize = 8192; // Safe chunk size for String.fromCharCode
    let binaryString = '';
    for (let i = 0; i < bytes.length; i += chunkSize) {
      const chunk = bytes.slice(i, i + chunkSize);
      binaryString += String.fromCharCode(...chunk);
    }
    return btoa(binaryString);
  } else {
    throw new FileSystemError(
      `Unrecognized bytes type in readFile ${typeof bytes}`
    );
  }
};

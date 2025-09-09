import type {
  WasmTonkCore,
  WasmVfs,
  WasmRepo,
  WasmBundle,
} from './tonk_core.js';

/**
 * Metadata for a file or directory node in the virtual file system
 */
export interface NodeMetadata {
  /** Type of the node - either 'directory' or 'document' */
  nodeType: 'directory' | 'document';
  /** Creation timestamp */
  createdAt: Date;
  /** Last modification timestamp */
  modifiedAt: Date;
}

/**
 * Entry in a directory listing
 */
export interface DirectoryEntry {
  /** Name of the file or directory */
  name: string;
  /** Type of the entry */
  type: 'directory' | 'document';
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
  rootId: String;
  entrypoints: String[];
  networkUris: String[];
  xNotes?: String;
  xVendor?: Object;
}

/**
 * Configuration options for Tonk initialization
 */
export interface TonkConfig {
  /** Custom path to the WASM module */
  wasmPath?: string;
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

/**
 * Virtual file system with CRDT-based synchronization.
 *
 * Provides a file-like interface for storing and retrieving documents
 * that are automatically synchronized across peers.
 *
 * @example
 * ```typescript
 * const tonk = await TonkCore.create();
 * const vfs = await tonk.vfs;
 *
 * // Create a file
 * await vfs.createFile('/notes/todo.md', '# My Todo List');
 *
 * // Read it back
 * const content = await vfs.readFile('/notes/todo.md');
 * ```
 */
export class VirtualFileSystem {
  #wasm: WasmVfs;

  /** @internal */
  constructor(wasm: WasmVfs) {
    this.#wasm = wasm;
  }

  /**
   * Create a new file with the given content.
   *
   * @param path - Absolute path where the file should be created
   * @param content - Content to write to the file (string or binary data)
   * @throws {FileSystemError} If the file already exists or path is invalid
   *
   * @example
   * ```typescript
   * // Create a text file
   * await vfs.createFile('/hello.txt', 'Hello, World!');
   *
   * // Create a binary file
   * const imageData = new Uint8Array([...]);
   * await vfs.createFile('/image.png', imageData);
   * ```
   */
  async createFile(path: string, content: string): Promise<void> {
    try {
      await this.#wasm.createFile(path, content);
    } catch (error) {
      throw new FileSystemError(`Failed to create file at ${path}: ${error}`);
    }
  }

  /**
   * Read the contents of a file.
   *
   * @param path - Absolute path to the file
   * @returns The file contents as a string
   * @throws {FileSystemError} If the file doesn't exist or can't be read
   *
   * @example
   * ```typescript
   * const content = await vfs.readFile('/notes/todo.md');
   * console.log(content);
   * ```
   */
  async readFile(path: string): Promise<string> {
    try {
      const result = await this.#wasm.readFile(path);
      if (result === null) {
        throw new FileSystemError(`File not found: ${path}`);
      }
      return result;
    } catch (error) {
      if (error instanceof FileSystemError) throw error;
      throw new FileSystemError(`Failed to read file at ${path}: ${error}`);
    }
  }

  /**
   * Update an existing file with the given content.
   *
   * @param path - Absolute path of the file to update
   * @param content - Content to write to the file (string or binary data)
   * @returns true if the file was updated, false if it didn't exist
   * @throws {FileSystemError} If the path is invalid
   *
   * @example
   * ```typescript
   * // Create a text file
   * await vfs.createFile('/hello.txt', 'Hello, World!');
   *
   * // Overwrite it
   * await vfs.updateFile('/hello.txt', 'See you later!');
   * ```
   */
  async updateFile(
    path: string,
    content: string | Uint8Array
  ): Promise<boolean> {
    try {
      return await this.#wasm.updateFile(path, content);
    } catch (error) {
      throw new FileSystemError(`Failed to update file at ${path}: ${error}`);
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
   * Create a new directory.
   *
   * @param path - Absolute path where the directory should be created
   * @throws {FileSystemError} If the directory already exists or path is invalid
   *
   * @example
   * ```typescript
   * await vfs.createDirectory('/projects/my-app');
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
   * const entries = await vfs.listDirectory('/projects');
   * for (const entry of entries) {
   *   console.log(`${entry.type}: ${entry.name}`);
   * }
   * ```
   */
  async listDirectory(path: string): Promise<DirectoryEntry[]> {
    try {
      const entries = await this.#wasm.listDirectory(path);
      return entries.map((entry: any) => ({
        name: entry.name,
        type: entry.type as 'directory' | 'document',
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
   * Get metadata for a file or directory.
   *
   * @param path - Absolute path to the file or directory
   * @returns Metadata object or null if the path doesn't exist
   * @throws {FileSystemError} If the metadata can't be retrieved
   *
   * @example
   * ```typescript
   * const metadata = await vfs.getMetadata('/notes/todo.md');
   * if (metadata) {
   *   console.log(`Type: ${metadata.nodeType}`);
   *   console.log(`Created: ${metadata.createdAt}`);
   *   console.log(`Modified: ${metadata.modifiedAt}`);
   * }
   * ```
   */
  async getMetadata(path: string): Promise<NodeMetadata | null> {
    try {
      const result = await this.#wasm.getMetadata(path);
      if (result === null) return null;

      return {
        nodeType: result.node_type as 'directory' | 'document',
        createdAt: new Date(result.created_at * 1000),
        modifiedAt: new Date(result.modified_at * 1000),
      };
    } catch (error) {
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
   * const watcher = await vfs.watchFile('/text.txt', docState => {
   *   console.log('Document changed:', docState);
   * });
   * ```
   */
  async watchFile(
    path: string,
    callback: Function
  ): Promise<DocumentWatcher | null> {
    try {
      const result = await this.#wasm.watchDocument(path, callback);
      if (result === null) return null;

      return result;
    } catch (error) {
      throw new FileSystemError(
        `Failed to watch file at path ${path}: ${error}`
      );
    }
  }

  /**
   * Watch a directory for changes at the specified path
   *
   * @param path - Absolute path to the directory
   * @param callback - Callback to run on change events
   * @returns A DocumentWatcher for the specified path
   *
   * @example
   * ```typescript
   * const watcher = await vfs.watchDirecotry('/documents', docState => {
   *   console.log('Directory changed:', docState);
   * });
   * ```
   */
  async watchDirectory(
    path: string,
    callback: Function
  ): Promise<DocumentWatcher | null> {
    try {
      const result = await this.#wasm.watchDirectory(path, callback);
      if (result === null) return null;

      return result;
    } catch (error) {
      throw new FileSystemError(
        `Failed to watch directory at path ${path}: ${error}`
      );
    }
  }
}

/**
 * Repository for managing Automerge documents.
 *
 * Provides low-level access to CRDT documents for advanced use cases.
 * Most users should use the VirtualFileSystem instead.
 *
 * @example
 * ```typescript
 * const repo = await tonk.repo;
 * const docId = await repo.createDocument('{"title": "My Document"}');
 * const content = await repo.findDocument(docId);
 * ```
 */
export class Repository {
  #wasm: WasmRepo;

  /** @internal */
  constructor(wasm: WasmRepo) {
    this.#wasm = wasm;
  }

  /**
   * Get the peer ID of this repository.
   *
   * @returns The peer ID as a string
   */
  getPeerId(): Promise<string> {
    return this.#wasm.getPeerId();
  }

  /**
   * Create a new Automerge document with the given content.
   *
   * @param content - Initial content for the document (as JSON string)
   * @returns The ID of the created document
   * @throws {Error} If document creation fails
   */
  async createDocument(content: string): Promise<string> {
    return this.#wasm.createDocument(content);
  }

  /**
   * Find and retrieve a document by its ID.
   *
   * @param docId - The document ID to search for
   * @returns The document content as a string, or null if not found
   * @throws {Error} If the document ID is invalid
   */
  async findDocument(docId: string): Promise<string | null> {
    const result = await this.#wasm.findDocument(docId);
    return result === null ? null : result;
  }
}

// TODO: update docs for bundles
/**
 * Bundle for storing and retrieving binary data in a ZIP-like format.
 *
 * Bundles provide efficient storage and retrieval of key-value pairs
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
  async getRootId(): Promise<String> {
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
   * @returns Array of matching entries
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
      throw new BundleError(`Failed to retrive manifest: ${error}`);
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
 * Main synchronization engine for Tonk.
 *
 * The TonkCore manages peer-to-peer synchronization, WebSocket connections,
 * and provides access to the virtual file system and repository.
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
 * await tonk.connectWebsocket('ws://localhost:8080');
 *
 * // Access the virtual file system
 * const vfs = await tonk.vfs;
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
   * @param wasmModule - WASM module functions (for lazy loading)
   * @returns A new TonkCore instance
   * @throws {Error} If Tonk creation fails or WASM not initialized
   */
  static async create(wasmModule?: any): Promise<TonkCore> {
    const { create_tonk } = wasmModule || (await import('./tonk_core.js'));
    const wasm = await create_tonk();
    return new TonkCore(wasm);
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
   * @param wasmModule - WASM module functions (for lazy loading)
   * @returns A new TonkCore instance
   * @throws {Error} If Tonk creation fails, bundle is invalid, or WASM not initialized
   */
  static async fromBundle(bundle: Bundle, wasmModule?: any): Promise<TonkCore> {
    const { create_tonk_from_bundle } =
      wasmModule || (await import('./tonk_core.js'));
    const wasm = await create_tonk_from_bundle(bundle);
    return new TonkCore(wasm);
  }

  /**
   * Create a new Tonk Core from bundle data
   *
   * @param data - The Bundle data from which to load
   * @param wasmModule - WASM module functions (for lazy loading)
   * @returns A new TonkCore instance
   * @throws {Error} If Tonk creation fails, bundle is invalid, or WASM not initialized
   */
  static async fromBytes(
    data: Uint8Array,
    wasmModule?: any
  ): Promise<TonkCore> {
    const { create_tonk_from_bytes } =
      wasmModule || (await import('./tonk_core.js'));
    const wasm = await create_tonk_from_bytes(data);
    return new TonkCore(wasm);
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
   * Get the virtual file system instance.
   *
   * @returns The VirtualFileSystem instance
   */
  async getVfs(): Promise<VirtualFileSystem> {
    return this.#wasm.getVfs().then((wasm: any) => new VirtualFileSystem(wasm));
  }

  /**
   * Get the repository instance.
   *
   * @returns The Repository instance
   */
  async getRepo(): Promise<Repository> {
    return this.#wasm.getRepo().then((wasm: any) => new Repository(wasm));
  }

  /**
   * Connect to a WebSocket server for real-time synchronization.
   *
   * @param url - WebSocket URL to connect to
   * @throws {ConnectionError} If connection fails
   *
   * @example
   * ```typescript
   * await tonk.connectWebsocket('ws://sync.example.com:8080');
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
   * Serialize the Tonk Core to bundle binary data.
   *
   * @returns The serialized bundle data
   * @throws {TonkError} If serialization fails
   */
  async toBytes(): Promise<Uint8Array> {
    try {
      return await this.#wasm.toBytes();
    } catch (error) {
      throw new TonkError(`Failed to serialize to bundle data: ${error}`);
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
     * @param bundle - The Bundle data from which to load
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

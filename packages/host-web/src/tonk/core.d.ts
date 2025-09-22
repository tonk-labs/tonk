import type { WasmBundle } from './tonk_core.js';
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
 * Base error class for all Tonk-related errors
 */
export declare class TonkError extends Error {
    code?: string | undefined;
    constructor(message: string, code?: string | undefined);
}
/**
 * Error thrown when connection operations fail
 */
export declare class ConnectionError extends TonkError {
    constructor(message: string);
}
/**
 * Error thrown when file system operations fail
 */
export declare class FileSystemError extends TonkError {
    constructor(message: string);
}
/**
 * Error thrown when bundle operations fail
 */
export declare class BundleError extends TonkError {
    constructor(message: string);
}
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
export declare class Bundle {
    #private;
    /** @internal */
    constructor(wasm: WasmBundle);
    /**
     * Create a bundle from existing data.
     *
     * @param data - Binary data representing a serialized bundle
     * @param wasmModule - WASM module functions (for lazy loading)
     * @returns A new Bundle instance
     * @throws {BundleError} If the data is invalid, corrupted, or WASM not initialized
     */
    static fromBytes(data: Uint8Array, wasmModule?: any): Promise<Bundle>;
    /**
     * Retrieve the root ID from the bundle
     *
     * @returns The root ID
     * @throws {BundleError} If the operation fails
     */
    getRootId(): Promise<String>;
    /**
     * Retrieve a value from the bundle.
     *
     * @param key - The key to look up
     * @returns The stored data, or null if the key doesn't exist
     * @throws {BundleError} If the operation fails
     */
    get(key: string): Promise<Uint8Array | null>;
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
    getPrefix(prefix: string): Promise<BundleEntry[]>;
    /**
     * List all keys in the bundle.
     *
     * @returns Array of all keys
     * @throws {BundleError} If the operation fails
     */
    listKeys(): Promise<string[]>;
    /**
     * Retrieve the bundle manifest
     *
     * @returns Manifest as JSON
     * @throws {BundleError} If the operation fails
     */
    getManifest(): Promise<Manifest>;
    /**
     * Serialize the bundle to binary data.
     *
     * @returns The serialized bundle data
     * @throws {BundleError} If serialization fails
     */
    toBytes(): Promise<Uint8Array>;
    /**
     * Free the WASM memory associated with this bundle.
     * Call this when you're done with the bundle to prevent memory leaks.
     */
    free(): void;
}
/**
 * Main synchronization engine for Tonk.
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
 * await tonk.connectWebsocket('ws://localhost:8080');
 * ```
 */
export declare class TonkCore {
    #private;
    /** @internal */
    private constructor();
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
    static create(config?: TonkConfig, wasmModule?: any): Promise<TonkCore>;
    /**
     * Create a new Tonk Core with a specific peer ID.
     *
     * @param peerId - The peer ID to use
     * @param wasmModule - WASM module functions (for lazy loading)
     * @returns A new TonkCore instance
     * @throws {Error} If Tonk creation fails, peer ID is invalid, or WASM not initialized
     */
    static createWithPeerId(peerId: string, wasmModule?: any): Promise<TonkCore>;
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
    static fromBundle(bundle: Bundle, config?: TonkConfig, wasmModule?: any): Promise<TonkCore>;
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
    static fromBytes(data: Uint8Array, config?: TonkConfig, wasmModule?: any): Promise<TonkCore>;
    /**
     * Get the peer ID of this Tonk Core
     *
     * @returns The peer ID as a string
     */
    getPeerId(): Promise<string>;
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
    connectWebsocket(url: string): Promise<void>;
    /**
     * Serialize the Tonk Core to bundle binary data.
     *
     * @returns The serialized bundle data
     * @throws {TonkError} If serialization fails
     */
    toBytes(): Promise<Uint8Array>;
    /**
     * Create a new file with the given content.
     *
     * @param path - Absolute path where the file should be created
     * @param content - Content to write to the file (any JSON-serializable value)
     * @throws {FileSystemError} If the file already exists or path is invalid
     *
     * @example
     * ```typescript
     * // Create a text file with a string
     * await createFile('/hello.txt', 'Hello, World!');
     *
     * // Create a JSON file with an object
     * await createFile('/config.json', { theme: 'dark', fontSize: 14 });
     *
     * // Create a file with an array
     * await createFile('/data.json', [1, 2, 3, 4, 5]);
     * ```
     */
    createFile(path: string, content: any): Promise<void>;
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
    createFileWithBytes(path: string, content: any, bytes: Uint8Array): Promise<void>;
    /**
     * Read the contents of a file.
     *
     * @param path - Absolute path to the file
     * @returns The file contents as a string
     * @throws {FileSystemError} If the file doesn't exist or can't be read
     *
     * @example
     * ```typescript
     * const content = await readFile('/notes/todo.md');
     * console.log(content);
     * ```
     */
    readFile(path: string): Promise<any>;
    /**
     * Update an existing file with the given content.
     *
     * @param path - Absolute path of the file to update
     * @param content - Content to write to the file (any JSON-serializable value)
     * @returns true if the file was updated, false if it didn't exist
     * @throws {FileSystemError} If the path is invalid
     *
     * @example
     * ```typescript
     * // Create a text file
     * await createFile('/hello.txt', 'Hello, World!');
     *
     * // Overwrite it with a string
     * await updateFile('/hello.txt', 'See you later!');
     *
     * // Update with an object
     * await updateFile('/config.json', { theme: 'light', fontSize: 16 });
     * ```
     */
    updateFile(path: string, content: any): Promise<boolean>;
    /**
     * Update an existing file with the given content.
     *
     * @param path - Absolute path of the file to update
     * @param content - Content to write to the file (any JSON-serializable value)
     * @returns true if the file was updated, false if it didn't exist
     * @throws {FileSystemError} If the path is invalid
     *
     * @example
     * ```typescript
     * // Create a text file
     * await createFile('/hello.txt', 'Hello, World!');
     *
     * // Overwrite it with a string
     * await updateFile('/hello.txt', 'See you later!');
     *
     * // Update with an object
     * await updateFile('/config.json', { theme: 'light', fontSize: 16 });
     * ```
     */
    updateFileWithBytes(path: string, content: any, bytes: Uint8Array): Promise<boolean>;
    /**
     * Delete a file.
     *
     * @param path - Absolute path to the file
     * @returns true if the file was deleted, false if it didn't exist
     * @throws {FileSystemError} If the deletion fails
     */
    deleteFile(path: string): Promise<boolean>;
    /**
     * Create a new directory.
     *
     * @param path - Absolute path where the directory should be created
     * @throws {FileSystemError} If the directory already exists or path is invalid
     *
     * @example
     * ```typescript
     * await createDirectory('/projects/my-app');
     * ```
     */
    createDirectory(path: string): Promise<void>;
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
     *   console.log(`${entry.type}: ${entry.name}`);
     * }
     * ```
     */
    listDirectory(path: string): Promise<DirectoryEntry[]>;
    /**
     * Check if a file or directory exists at the given path.
     *
     * @param path - Absolute path to check
     * @returns true if something exists at the path, false otherwise
     */
    exists(path: string): Promise<boolean>;
    /**
     * Get metadata for a file or directory.
     *
     * @param path - Absolute path to the file or directory
     * @returns Metadata object or null if the path doesn't exist
     * @throws {FileSystemError} If the metadata can't be retrieved
     *
     * @example
     * ```typescript
     * const metadata = await getMetadata('/notes/todo.md');
     * if (metadata) {
     *   console.log(`Type: ${metadata.nodeType}`);
     *   console.log(`Created: ${metadata.createdAt}`);
     *   console.log(`Modified: ${metadata.modifiedAt}`);
     * }
     * ```
     */
    getMetadata(path: string): Promise<NodeMetadata | null>;
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
    watchFile(path: string, callback: Function): Promise<DocumentWatcher | null>;
    /**
     * Watch a directory for changes at the specified path
     *
     * @param path - Absolute path to the directory
     * @param callback - Callback to run on change events
     * @returns A DocumentWatcher for the specified path
     *
     * @example
     * ```typescript
     * const watcher = await watchDirecotry('/documents', docState => {
     *   console.log('Directory changed:', docState);
     * });
     * ```
     */
    watchDirectory(path: string, callback: Function): Promise<DocumentWatcher | null>;
    /**
     * Free the WASM memory associated with this Tonk Core.
     * Call this when you're done with the Tonk to prevent memory leaks.
     */
    free(): void;
}
/**
 * Factory functions that can work with either direct or lazy-loaded WASM modules
 */
export declare function createFactoryFunctions(wasmModule?: any): {
    /**
     * Create a new Tonk Core with an auto-generated peer ID.
     *
     * @returns A new TonkCore instance
     * @throws {Error} If Tonk creation fails
     */
    createTonk: () => Promise<TonkCore>;
    /**
     * Create a new Tonk Core with a specific peer ID.
     *
     * @param peerId - The peer ID to use
     * @returns A new TonkCore instance
     * @throws {Error} If Tonk creation fails or peer ID is invalid
     */
    createTonkWithPeerId: (peerId: string) => Promise<TonkCore>;
    /**
     * Create a Tonk Core from an existing bundle
     * @param bundle - The Bundle instance from which to load
     * @returns A new TonkCore instance
     * @throws {Error} If Tonk creation fails or bundle is invalid
     */
    createTonkFromBundle: (bundle: Bundle) => Promise<TonkCore>;
    /**
     * Create a Tonk Core from bundle data
     * @param bundle - The Bundle data from which to load
     * @returns A new TonkCore instance
     * @throws {Error} If Tonk creation fails or bundle is invalid
     */
    createTonkFromBytes: (data: Uint8Array) => Promise<TonkCore>;
    /**
     * Create a bundle from existing data.
     *
     * @param data - Binary data representing a serialized bundle
     * @returns A new Bundle instance
     * @throws {BundleError} If the data is invalid or corrupted
     */
    createBundleFromBytes: (data: Uint8Array) => Promise<Bundle>;
};
//# sourceMappingURL=core.d.ts.map
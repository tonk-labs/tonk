var __classPrivateFieldSet = (this && this.__classPrivateFieldSet) || function (receiver, state, value, kind, f) {
    if (kind === "m") throw new TypeError("Private method is not writable");
    if (kind === "a" && !f) throw new TypeError("Private accessor was defined without a setter");
    if (typeof state === "function" ? receiver !== state || !f : !state.has(receiver)) throw new TypeError("Cannot write private member to an object whose class did not declare it");
    return (kind === "a" ? f.call(receiver, value) : f ? f.value = value : state.set(receiver, value)), value;
};
var __classPrivateFieldGet = (this && this.__classPrivateFieldGet) || function (receiver, state, kind, f) {
    if (kind === "a" && !f) throw new TypeError("Private accessor was defined without a getter");
    if (typeof state === "function" ? receiver !== state || !f : !state.has(receiver)) throw new TypeError("Cannot read private member from an object whose class did not declare it");
    return kind === "m" ? f : kind === "a" ? f.call(receiver) : f ? f.value : state.get(receiver);
};
var _Bundle_wasm, _TonkCore_wasm;
/**
 * Base error class for all Tonk-related errors
 */
export class TonkError extends Error {
    constructor(message, code) {
        super(message);
        this.code = code;
        this.name = 'TonkError';
    }
}
/**
 * Error thrown when connection operations fail
 */
export class ConnectionError extends TonkError {
    constructor(message) {
        super(message, 'CONNECTION_ERROR');
        this.name = 'ConnectionError';
    }
}
/**
 * Error thrown when file system operations fail
 */
export class FileSystemError extends TonkError {
    constructor(message) {
        super(message, 'FILESYSTEM_ERROR');
        this.name = 'FileSystemError';
    }
}
/**
 * Error thrown when bundle operations fail
 */
export class BundleError extends TonkError {
    constructor(message) {
        super(message, 'BUNDLE_ERROR');
        this.name = 'BundleError';
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
    /** @internal */
    constructor(wasm) {
        _Bundle_wasm.set(this, void 0);
        __classPrivateFieldSet(this, _Bundle_wasm, wasm, "f");
    }
    /**
     * Create a bundle from existing data.
     *
     * @param data - Binary data representing a serialized bundle
     * @param wasmModule - WASM module functions (for lazy loading)
     * @returns A new Bundle instance
     * @throws {BundleError} If the data is invalid, corrupted, or WASM not initialized
     */
    static async fromBytes(data, wasmModule) {
        try {
            const { create_bundle_from_bytes } = wasmModule || (await import('./tonk_core.js'));
            return new Bundle(create_bundle_from_bytes(data));
        }
        catch (error) {
            throw new BundleError(`Failed to create bundle from bytes: ${error}`);
        }
    }
    /**
     * Retrieve the root ID from the bundle
     *
     * @returns The root ID
     * @throws {BundleError} If the operation fails
     */
    async getRootId() {
        try {
            const rootId = await __classPrivateFieldGet(this, _Bundle_wasm, "f").getRootId();
            return rootId;
        }
        catch (error) {
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
    async get(key) {
        try {
            const result = await __classPrivateFieldGet(this, _Bundle_wasm, "f").get(key);
            return result === null ? null : result;
        }
        catch (error) {
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
    async getPrefix(prefix) {
        try {
            const results = await __classPrivateFieldGet(this, _Bundle_wasm, "f").getPrefix(prefix);
            return results.map((entry) => ({
                key: entry.key,
                value: entry.value,
            }));
        }
        catch (error) {
            throw new BundleError(`Failed to get prefix ${prefix}: ${error}`);
        }
    }
    /**
     * List all keys in the bundle.
     *
     * @returns Array of all keys
     * @throws {BundleError} If the operation fails
     */
    async listKeys() {
        try {
            return await __classPrivateFieldGet(this, _Bundle_wasm, "f").listKeys();
        }
        catch (error) {
            throw new BundleError(`Failed to list keys: ${error}`);
        }
    }
    /**
     * Retrieve the bundle manifest
     *
     * @returns Manifest as JSON
     * @throws {BundleError} If the operation fails
     */
    async getManifest() {
        try {
            return await __classPrivateFieldGet(this, _Bundle_wasm, "f").getManifest();
        }
        catch (error) {
            throw new BundleError(`Failed to retrive manifest: ${error}`);
        }
    }
    /**
     * Serialize the bundle to binary data.
     *
     * @returns The serialized bundle data
     * @throws {BundleError} If serialization fails
     */
    async toBytes() {
        try {
            return await __classPrivateFieldGet(this, _Bundle_wasm, "f").toBytes();
        }
        catch (error) {
            throw new BundleError(`Failed to serialize bundle: ${error}`);
        }
    }
    /**
     * Free the WASM memory associated with this bundle.
     * Call this when you're done with the bundle to prevent memory leaks.
     */
    free() {
        __classPrivateFieldGet(this, _Bundle_wasm, "f").free();
    }
}
_Bundle_wasm = new WeakMap();
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
export class TonkCore {
    /** @internal */
    constructor(wasm) {
        _TonkCore_wasm.set(this, void 0);
        __classPrivateFieldSet(this, _TonkCore_wasm, wasm, "f");
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
    static async create(config, wasmModule) {
        const module = wasmModule || (await import('./tonk_core.js'));
        if (config?.peerId && config?.storage) {
            const { create_tonk_with_config } = module;
            const wasm = await create_tonk_with_config(config.peerId, config.storage.type === 'indexeddb');
            return new TonkCore(wasm);
        }
        else if (config?.peerId) {
            const { create_tonk_with_peer_id } = module;
            const wasm = await create_tonk_with_peer_id(config.peerId);
            return new TonkCore(wasm);
        }
        else if (config?.storage) {
            const { create_tonk_with_storage } = module;
            const wasm = await create_tonk_with_storage(config.storage.type === 'indexeddb');
            return new TonkCore(wasm);
        }
        else {
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
    static async createWithPeerId(peerId, wasmModule) {
        const { create_tonk_with_peer_id } = wasmModule || (await import('./tonk_core.js'));
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
    static async fromBundle(bundle, config, wasmModule) {
        const module = wasmModule || (await import('./tonk_core.js'));
        if (config?.storage) {
            const { create_tonk_from_bundle_with_storage } = module;
            const wasm = await create_tonk_from_bundle_with_storage(bundle, config.storage.type === 'indexeddb');
            return new TonkCore(wasm);
        }
        else {
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
    static async fromBytes(data, config, wasmModule) {
        const module = wasmModule || (await import('./tonk_core.js'));
        if (config?.storage) {
            const { create_tonk_from_bytes_with_storage } = module;
            const wasm = await create_tonk_from_bytes_with_storage(data, config.storage.type === 'indexeddb');
            return new TonkCore(wasm);
        }
        else {
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
    getPeerId() {
        return __classPrivateFieldGet(this, _TonkCore_wasm, "f").getPeerId();
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
    async connectWebsocket(url) {
        try {
            await __classPrivateFieldGet(this, _TonkCore_wasm, "f").connectWebsocket(url);
        }
        catch (error) {
            throw new ConnectionError(`Failed to connect to ${url}: ${error}`);
        }
    }
    /**
     * Serialize the Tonk Core to bundle binary data.
     *
     * @returns The serialized bundle data
     * @throws {TonkError} If serialization fails
     */
    async toBytes() {
        try {
            return await __classPrivateFieldGet(this, _TonkCore_wasm, "f").toBytes();
        }
        catch (error) {
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
    async createFile(path, content) {
        try {
            await __classPrivateFieldGet(this, _TonkCore_wasm, "f").createFile(path, content);
        }
        catch (error) {
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
    async createFileWithBytes(path, content, bytes) {
        try {
            await __classPrivateFieldGet(this, _TonkCore_wasm, "f").createFileWithBytes(path, content, bytes);
        }
        catch (error) {
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
     * const content = await readFile('/notes/todo.md');
     * console.log(content);
     * ```
     */
    async readFile(path) {
        try {
            const result = await __classPrivateFieldGet(this, _TonkCore_wasm, "f").readFile(path);
            if (result === null) {
                throw new FileSystemError(`File not found: ${path}`);
            }
            return result;
        }
        catch (error) {
            if (error instanceof FileSystemError)
                throw error;
            throw new FileSystemError(`Failed to read file at ${path}: ${error}`);
        }
    }
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
    async updateFile(path, content) {
        try {
            return await __classPrivateFieldGet(this, _TonkCore_wasm, "f").updateFile(path, content);
        }
        catch (error) {
            throw new FileSystemError(`Failed to update file at ${path}: ${error}`);
        }
    }
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
    async updateFileWithBytes(path, content, bytes) {
        try {
            return await __classPrivateFieldGet(this, _TonkCore_wasm, "f").updateFileWithBytes(path, content, bytes);
        }
        catch (error) {
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
    async deleteFile(path) {
        try {
            return await __classPrivateFieldGet(this, _TonkCore_wasm, "f").deleteFile(path);
        }
        catch (error) {
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
     * await createDirectory('/projects/my-app');
     * ```
     */
    async createDirectory(path) {
        try {
            await __classPrivateFieldGet(this, _TonkCore_wasm, "f").createDirectory(path);
        }
        catch (error) {
            throw new FileSystemError(`Failed to create directory at ${path}: ${error}`);
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
     *   console.log(`${entry.type}: ${entry.name}`);
     * }
     * ```
     */
    async listDirectory(path) {
        try {
            const entries = await __classPrivateFieldGet(this, _TonkCore_wasm, "f").listDirectory(path);
            return entries.map((entry) => ({
                name: entry.name,
                type: entry.type,
            }));
        }
        catch (error) {
            throw new FileSystemError(`Failed to list directory at ${path}: ${error}`);
        }
    }
    /**
     * Check if a file or directory exists at the given path.
     *
     * @param path - Absolute path to check
     * @returns true if something exists at the path, false otherwise
     */
    async exists(path) {
        try {
            return await __classPrivateFieldGet(this, _TonkCore_wasm, "f").exists(path);
        }
        catch (error) {
            throw new FileSystemError(`Failed to check existence of ${path}: ${error}`);
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
     * const metadata = await getMetadata('/notes/todo.md');
     * if (metadata) {
     *   console.log(`Type: ${metadata.nodeType}`);
     *   console.log(`Created: ${metadata.createdAt}`);
     *   console.log(`Modified: ${metadata.modifiedAt}`);
     * }
     * ```
     */
    async getMetadata(path) {
        try {
            const result = await __classPrivateFieldGet(this, _TonkCore_wasm, "f").getMetadata(path);
            if (result === null)
                return null;
            return {
                nodeType: result.node_type,
                createdAt: new Date(result.created_at * 1000),
                modifiedAt: new Date(result.modified_at * 1000),
            };
        }
        catch (error) {
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
    async watchFile(path, callback) {
        try {
            const result = await __classPrivateFieldGet(this, _TonkCore_wasm, "f").watchDocument(path, callback);
            if (result === null)
                return null;
            return result;
        }
        catch (error) {
            throw new FileSystemError(`Failed to watch file at path ${path}: ${error}`);
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
     * const watcher = await watchDirecotry('/documents', docState => {
     *   console.log('Directory changed:', docState);
     * });
     * ```
     */
    async watchDirectory(path, callback) {
        try {
            const result = await __classPrivateFieldGet(this, _TonkCore_wasm, "f").watchDirectory(path, callback);
            if (result === null)
                return null;
            return result;
        }
        catch (error) {
            throw new FileSystemError(`Failed to watch directory at path ${path}: ${error}`);
        }
    }
    /**
     * Free the WASM memory associated with this Tonk Core.
     * Call this when you're done with the Tonk to prevent memory leaks.
     */
    free() {
        __classPrivateFieldGet(this, _TonkCore_wasm, "f").free();
    }
}
_TonkCore_wasm = new WeakMap();
/**
 * Factory functions that can work with either direct or lazy-loaded WASM modules
 */
export function createFactoryFunctions(wasmModule) {
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
        createTonkWithPeerId: (peerId) => TonkCore.createWithPeerId(peerId, wasmModule),
        /**
         * Create a Tonk Core from an existing bundle
         * @param bundle - The Bundle instance from which to load
         * @returns A new TonkCore instance
         * @throws {Error} If Tonk creation fails or bundle is invalid
         */
        createTonkFromBundle: (bundle) => TonkCore.fromBundle(bundle, wasmModule),
        /**
         * Create a Tonk Core from bundle data
         * @param bundle - The Bundle data from which to load
         * @returns A new TonkCore instance
         * @throws {Error} If Tonk creation fails or bundle is invalid
         */
        createTonkFromBytes: (data) => TonkCore.fromBytes(data, wasmModule),
        /**
         * Create a bundle from existing data.
         *
         * @param data - Binary data representing a serialized bundle
         * @returns A new Bundle instance
         * @throws {BundleError} If the data is invalid or corrupted
         */
        createBundleFromBytes: (data) => Bundle.fromBytes(data, wasmModule),
    };
}

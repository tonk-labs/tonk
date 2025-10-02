import type {
  Chunk,
  StorageAdapterInterface,
  StorageKey,
} from '@automerge/automerge-repo';
import JSZip from 'jszip';

/// Version information for the bundle
export interface Version {
  major: number;
  minor: number;
}

/// Manifest structure for bundle metadata
export interface Manifest {
  manifestVersion: number;
  version: Version;
  rootId: string;
  entrypoints: string[];
  networkUris: string[];
  xNotes?: string;
  xVendor?: any;
}

/**
 * BundleStorageAdapter - A storage adapter that loads initial data from a bundle
 * and stores updates in memory. Files are loaded from the bundle on-demand when requested.
 * Implements the same interface as InMemoryStorageAdapter but with bundle support for initial state loading.
 */
export class BundleStorageAdapter implements StorageAdapterInterface {
  #bundleBytes: Uint8Array | null = null;
  #bundleZip: JSZip | null = null;
  #memoryData: Map<string, Uint8Array> = new Map();
  #manifest: Manifest | null = null;
  #hasUnsavedChanges = false;

  /**
   * Create a new BundleStorageAdapter from bundle bytes
   */
  static async fromBundle(
    bundleBytes: Uint8Array
  ): Promise<BundleStorageAdapter> {
    const adapter = new BundleStorageAdapter();
    await adapter.loadBundle(bundleBytes);
    return adapter;
  }

  /**
   * Load bundle metadata and prepare ZIP structure for file access
   */
  private async loadBundle(bundleBytes: Uint8Array): Promise<void> {
    try {
      // Store bundle bytes for reference
      this.#bundleBytes = bundleBytes;

      // Load ZIP structure for file access
      const zip = new JSZip();
      const zipContent = await zip.loadAsync(bundleBytes);
      this.#bundleZip = zipContent;

      // Load and parse manifest
      const manifestFile = zipContent.file('manifest.json');
      if (!manifestFile) {
        throw new Error('manifest.json not found in bundle');
      }

      const manifestContent = await manifestFile.async('text');
      this.#manifest = JSON.parse(manifestContent) as Manifest;

      // Validate manifest version
      if (this.#manifest.manifestVersion !== 1) {
        throw new Error(
          `Unsupported manifest version: ${this.#manifest.manifestVersion}. Expected version 1.`
        );
      }

      // Count available files for logging
      const fileCount = Object.entries(zipContent.files).filter(
        ([path, file]) =>
          !(file as JSZip.JSZipObject).dir && path !== 'manifest.json'
      ).length;

      console.log(`Loaded bundle with ${fileCount} files available`);
    } catch (error) {
      throw new Error(`Failed to load bundle: ${error}`);
    }
  }

  /**
   * Convert StorageKey to string for internal map keys
   * This matches the Rust BundlePath.to_string() behavior exactly
   */
  private keyToString(key: StorageKey): string {
    // Filter out empty components and join with '/'
    // This matches BundlePath::new() behavior in Rust
    const filteredComponents = key.filter(s => s !== '');
    return filteredComponents.join('/');
  }

  /**
   * Convert string back to StorageKey
   * This matches the Rust BundlePath.parse_path() behavior exactly
   */
  private stringToKey(str: string): StorageKey {
    if (str === '' || str === '/') {
      return []; // Root path
    }

    // Split by '/', filter empty components, matching Rust behavior
    return str
      .replace(/^\/+/, '') // Remove leading slashes
      .split('/')
      .filter(s => s !== '');
  }

  /**
   * Check if a prefix matches a key
   */
  private isPrefixOf(prefix: StorageKey, candidate: StorageKey): boolean {
    return (
      prefix.length <= candidate.length &&
      prefix.every((segment, index) => segment === candidate[index])
    );
  }

  /**
   * Map bundle storage paths to Automerge expected paths
   * Converts: storage/4C/SBjwxmWCro5MfDrDUBtjJmZRYU/snapshot/bundle_export
   * To: 4CSBjwxmWCro5MfDrDUBtjJmZRYU/snapshot
   */
  private mapBundlePathToAutomergeKey(bundlePath: string): StorageKey | null {
    // Match pattern: storage/{prefix}/{rest_of_doc_id}/{type}/bundle_export
    const match = bundlePath.match(
      /^storage\/([^/]+)\/([^/]+)\/([^/]+)\/bundle_export$/
    );
    if (!match) {
      return null;
    }

    const [, prefix, restOfDocId, type] = match;
    const fullDocId = prefix + restOfDocId;

    return [fullDocId, type];
  }

  /**
   * Get the root document ID from the manifest
   */
  getRootId(): string | null {
    return this.#manifest?.rootId || null;
  }

  /**
   * Check if there are unsaved changes
   */
  hasUnsavedChanges(): boolean {
    return this.#hasUnsavedChanges;
  }

  /**
   * Load a file from the bundle by path
   */
  private async loadFromBundle(
    pathStr: string
  ): Promise<Uint8Array | undefined> {
    if (!this.#bundleZip) {
      return undefined;
    }

    console.log(pathStr);

    try {
      const file = this.#bundleZip.file(pathStr);
      if (!file) {
        return undefined;
      }

      return await file.async('uint8array');
    } catch (error) {
      console.warn(`Failed to load ${pathStr} from bundle:`, error);
      return undefined;
    }
  }

  // StorageAdapterInterface implementation

  /**
   * Load data by key - checks memory first, then loads from bundle
   */
  async load(key: StorageKey): Promise<Uint8Array | undefined> {
    const keyStr = this.keyToString(key);

    // Check memory updates first (they override bundle data)
    if (this.#memoryData.has(keyStr)) {
      return this.#memoryData.get(keyStr);
    }

    // Load from bundle if not in memory
    return await this.loadFromBundle(keyStr);
  }

  /**
   * Save data by key - stores in memory and marks as having unsaved changes
   */
  async save(key: StorageKey, data: Uint8Array): Promise<void> {
    const keyStr = this.keyToString(key);
    this.#memoryData.set(keyStr, data);
    this.#hasUnsavedChanges = true;
  }

  /**
   * Remove data by key - marks as removed in memory
   */
  async remove(key: StorageKey): Promise<void> {
    const keyStr = this.keyToString(key);
    this.#memoryData.delete(keyStr);
    this.#hasUnsavedChanges = true;
  }

  /**
   * Load all data matching a key prefix
   */
  async loadRange(keyPrefix: StorageKey): Promise<Chunk[]> {
    const result: Chunk[] = [];
    const prefixStr = this.keyToString(keyPrefix);

    // Check all files in bundle (without loading them yet)
    if (this.#bundleZip) {
      for (const [relativePath, file] of Object.entries(
        this.#bundleZip.files
      )) {
        const zipFile = file as JSZip.JSZipObject;
        if (!zipFile.dir && relativePath !== 'manifest.json') {
          const key = this.stringToKey(relativePath);
          // Also check if this is a bundle storage file that maps to the expected Automerge path
          const mappedKey = this.mapBundlePathToAutomergeKey(relativePath);
          const mappedMatches = mappedKey
            ? this.isPrefixOf(keyPrefix, mappedKey)
            : false;
          if (this.isPrefixOf(keyPrefix, key) || mappedMatches) {
            // Only include if not overridden by memory data
            if (!this.#memoryData.has(relativePath)) {
              // Load the data from bundle
              const data = await this.loadFromBundle(relativePath);
              if (data) {
                // Use the mapped key if we matched via mapping, otherwise use the original key
                const resultKey = mappedMatches && mappedKey ? mappedKey : key;
                result.push({ key: resultKey, data });
              }
            }
          }
        }
      }
    }

    // Check all memory data
    for (const [keyStr, data] of this.#memoryData.entries()) {
      const key = this.stringToKey(keyStr);
      if (this.isPrefixOf(keyPrefix, key)) {
        result.push({ key, data });
      }
    }

    return result;
  }

  /**
   * Remove all data matching a key prefix
   */
  async removeRange(keyPrefix: StorageKey): Promise<void> {
    // Mark all matching keys for removal in memory
    // For bundle files, we mark them as deleted by setting them to undefined in memory
    if (this.#bundleZip) {
      for (const [relativePath, file] of Object.entries(
        this.#bundleZip.files
      )) {
        const zipFile = file as JSZip.JSZipObject;
        if (!zipFile.dir && relativePath !== 'manifest.json') {
          const key = this.stringToKey(relativePath);
          if (this.isPrefixOf(keyPrefix, key)) {
            // Mark as deleted by removing from memory (if it was cached) or setting a deletion marker
            this.#memoryData.delete(relativePath);
          }
        }
      }
    }

    // Remove from memory data
    for (const [keyStr] of this.#memoryData.entries()) {
      const key = this.stringToKey(keyStr);
      if (this.isPrefixOf(keyPrefix, key)) {
        this.#memoryData.delete(keyStr);
      }
    }

    this.#hasUnsavedChanges = true;
  }

  /**
   * Create a new bundle with the current state (bundle data + memory updates)
   */
  async createBundle(newManifest?: Partial<Manifest>): Promise<Uint8Array> {
    const zip = new JSZip();

    // Create updated manifest
    const manifest: Manifest = {
      manifestVersion: 1,
      version: { major: 1, minor: 0 },
      rootId: this.#manifest?.rootId || 'unknown',
      entrypoints: [],
      networkUris: [],
      ...this.#manifest,
      ...newManifest,
    };

    // Add manifest.json to zip
    zip.file('manifest.json', JSON.stringify(manifest, null, 2));

    // Collect all current data (bundle data + memory updates)
    const allData = new Map<string, Uint8Array>();

    // Start with bundle data (load each file)
    if (this.#bundleZip) {
      for (const [relativePath, file] of Object.entries(
        this.#bundleZip.files
      )) {
        const zipFile = file as JSZip.JSZipObject;
        if (!zipFile.dir && relativePath !== 'manifest.json') {
          // Only load if not overridden by memory data
          if (!this.#memoryData.has(relativePath)) {
            const data = await this.loadFromBundle(relativePath);
            if (data) {
              allData.set(relativePath, data);
            }
          }
        }
      }
    }

    // Override with memory updates
    for (const [keyStr, data] of this.#memoryData.entries()) {
      allData.set(keyStr, data);
    }

    // Add all data to zip using the path format that matches Rust expectations
    for (const [pathStr, data] of allData.entries()) {
      // pathStr is already in the correct format for ZIP files
      // This ensures compatibility with Rust BundlePath.to_string() format
      zip.file(pathStr, data);
    }

    // Generate the ZIP bundle
    const bundleBytes = await zip.generateAsync({
      type: 'uint8array',
      compression: 'DEFLATE',
      compressionOptions: { level: 6 },
    });

    // Reset unsaved changes flag since we just created a bundle
    this.#hasUnsavedChanges = false;

    return bundleBytes;
  }

  /**
   * Create a slim bundle containing the manifest file and the storage folder
   * that matches the first two letters of the rootId
   * Useful for lightweight operations or when only metadata and root document data is needed
   */
  async createSlimBundle(newManifest?: Partial<Manifest>): Promise<Uint8Array> {
    const zip = new JSZip();

    if (!this.#manifest?.rootId) {
      throw new Error('we do not make up the root doc');
    }

    // Create updated manifest
    const manifest: Manifest = {
      ...this.#manifest,
      ...newManifest,
    };

    // Add manifest.json to zip
    zip.file('manifest.json', JSON.stringify(manifest, null, 2));

    // Get the first two letters of rootId for the storage folder prefix
    const rootIdPrefix = this.#manifest.rootId.substring(0, 2);
    const storageFolderPrefix = `storage/${rootIdPrefix}`;

    // Add all files from the storage folder that matches the rootId prefix
    // if (this.#bundleZip) {
    //   for (const [relativePath, file] of Object.entries(
    //     this.#bundleZip.files
    //   )) {
    //     const zipFile = file as JSZip.JSZipObject;
    //     if (
    //       !zipFile.dir &&
    //       relativePath !== 'manifest.json' &&
    //       relativePath.startsWith(storageFolderPrefix)
    //     ) {
    //       // Check if this file has been updated in memory
    //       if (this.#memoryData.has(relativePath)) {
    //         // Use the updated version from memory
    //         const data = this.#memoryData.get(relativePath);
    //         if (data) {
    //           zip.file(relativePath, data);
    //         }
    //       } else {
    //         // Load from bundle
    //         const data = await this.loadFromBundle(relativePath);
    //         if (data) {
    //           zip.file(relativePath, data);
    //         }
    //       }
    //     }
    //   }
    // }

    // Also add any memory-only files that match the storage folder prefix
    for (const [keyStr, data] of this.#memoryData.entries()) {
      if (keyStr.startsWith(storageFolderPrefix) && !zip.file(keyStr)) {
        zip.file(keyStr, data);
      }
    }

    // Generate the slim ZIP bundle
    const bundleBytes = await zip.generateAsync({
      type: 'uint8array',
      compression: 'DEFLATE',
      compressionOptions: { level: 6 },
    });

    return bundleBytes;
  }

  /**
   * Log current state for debugging
   */
  log(): void {
    console.log(`BundleStorageAdapter state:`);

    // Count available bundle files
    let bundleFileCount = 0;
    if (this.#bundleZip) {
      bundleFileCount = Object.entries(this.#bundleZip.files).filter(
        ([path, file]) =>
          !(file as JSZip.JSZipObject).dir && path !== 'manifest.json'
      ).length;
    }

    console.log(`  Bundle files available: ${bundleFileCount} items`);
    console.log(`  Memory updates: ${this.#memoryData.size} items`);
    console.log(`  Has unsaved changes: ${this.#hasUnsavedChanges}`);
    console.log(`  Root ID: ${this.getRootId()}`);

    if (this.#bundleZip && bundleFileCount > 0) {
      console.log(`  Bundle file paths:`);
      for (const [relativePath, file] of Object.entries(
        this.#bundleZip.files
      )) {
        const zipFile = file as JSZip.JSZipObject;
        if (!zipFile.dir && relativePath !== 'manifest.json') {
          console.log(`    ${relativePath}`);
        }
      }
    }

    if (this.#memoryData.size > 0) {
      console.log(`  Memory keys:`);
      for (const [key, value] of this.#memoryData.entries()) {
        console.log(`    ${key}: ${value.length} bytes`);
      }
    }
  }
}

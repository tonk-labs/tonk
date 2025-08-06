/**
 * ZipBundle - Concrete implementation of Bundle using JSZip
 */

import JSZip from 'jszip';
import { Bundle } from './bundle.js';
// Removed unused imports
import { validateBundleComprehensive } from '../parser/validation.js';
import {
  BundleManifestSchema,
  type BundleManifestType,
} from '../schemas/bundle.js';
import type {
  BundleManifest,
  BundleFile,
  BundleInfo,
  AddFileOptions,
  ParseOptions,
  SerializationOptions,
} from '../types/bundle.js';
import type {
  ValidationOptions,
  ValidationResult,
} from '../types/validation.js';
import {
  BundleParseError,
  FileNotFoundError,
  BundleValidationError,
} from '../types/errors.js';

/**
 * ZIP-based implementation of the Bundle interface
 */
export class ZipBundle extends Bundle {
  private zip: JSZip;
  private _manifest: BundleManifestType;
  private _sourceData: ArrayBuffer | null = null;

  /**
   * Create a new ZipBundle instance
   * @param zip - JSZip instance containing the bundle data
   * @param manifest - Validated bundle manifest
   * @param sourceData - Original source data (optional)
   */
  constructor(
    zip: JSZip,
    manifest: BundleManifestType,
    sourceData?: ArrayBuffer
  ) {
    super();
    this.zip = zip;
    this._manifest = manifest;
    this._sourceData = sourceData || null;
  }

  get manifest(): BundleManifest {
    return this._manifest;
  }

  get data(): ArrayBuffer | null {
    return this._sourceData;
  }

  // File Access Methods

  getFile(path: string): BundleFile | null {
    return this._manifest.files.find(file => file.path === path) || null;
  }

  async getFileData(path: string): Promise<ArrayBuffer | null> {
    // Convert virtual path to ZIP path (remove leading /)
    const zipPath = path.startsWith('/') ? path.slice(1) : path;
    const zipFile = this.zip.file(zipPath);

    if (!zipFile) {
      return null;
    }

    try {
      return await zipFile.async('arraybuffer');
    } catch (error) {
      throw new BundleParseError(
        `Failed to read file data for ${path}: ${error instanceof Error ? error.message : 'Unknown error'}`
      );
    }
  }

  hasFile(path: string): boolean {
    return this._manifest.files.some(file => file.path === path);
  }

  listFiles(): BundleFile[] {
    return [...this._manifest.files];
  }

  getFileCount(): number {
    return this._manifest.files.length;
  }

  // Entrypoint Methods

  getEntrypoint(name: string): string | null {
    return this._manifest.entrypoints[name] || null;
  }

  hasEntrypoint(name: string): boolean {
    return name in this._manifest.entrypoints;
  }

  listEntrypoints(): Record<string, string> {
    return { ...this._manifest.entrypoints };
  }

  getEntrypointNames(): string[] {
    return Object.keys(this._manifest.entrypoints);
  }

  // File Modification Methods

  async addFile(
    file: Omit<BundleFile, 'length'>,
    data: ArrayBuffer,
    options: AddFileOptions = {}
  ): Promise<void> {
    const {
      replace = false,
      compress = true,
      contentType,
      lastModified,
    } = options;

    // Check if file already exists
    if (this.hasFile(file.path) && !replace) {
      throw new BundleValidationError(
        `File ${file.path} already exists. Set replace=true to overwrite.`
      );
    }

    // Create the complete file metadata
    const completeFile: BundleFile = {
      ...file,
      length: data.byteLength,
      contentType: contentType || file.contentType,
      compressed: compress,
      lastModified: lastModified || new Date().toISOString(),
    };

    // Add to ZIP (convert virtual path to ZIP path)
    const zipPath = file.path.startsWith('/') ? file.path.slice(1) : file.path;
    this.zip.file(zipPath, data, {
      compression: compress ? 'DEFLATE' : 'STORE',
      date: lastModified ? new Date(lastModified) : new Date(),
    });

    // Update manifest
    if (replace) {
      const index = this._manifest.files.findIndex(f => f.path === file.path);
      if (index >= 0) {
        this._manifest.files[index] = completeFile;
      } else {
        this._manifest.files.push(completeFile);
      }
    } else {
      this._manifest.files.push(completeFile);
    }

    // Update manifest in ZIP
    await this.updateManifestInZip();
  }

  async updateFile(
    path: string,
    data: ArrayBuffer,
    contentType?: string
  ): Promise<void> {
    const existingFile = this.getFile(path);
    if (!existingFile) {
      throw new FileNotFoundError(`File ${path} not found`);
    }

    // Update file metadata
    const updatedFile: BundleFile = {
      ...existingFile,
      length: data.byteLength,
      contentType: contentType || existingFile.contentType,
      lastModified: new Date().toISOString(),
    };

    // Update in ZIP
    const zipPath = path.startsWith('/') ? path.slice(1) : path;
    this.zip.file(zipPath, data, {
      compression: existingFile.compressed ? 'DEFLATE' : 'STORE',
      date: new Date(),
    });

    // Update manifest
    const index = this._manifest.files.findIndex(f => f.path === path);
    if (index >= 0) {
      this._manifest.files[index] = updatedFile;
    }

    // Update manifest in ZIP
    await this.updateManifestInZip();
  }

  async removeFile(path: string): Promise<void> {
    if (!this.hasFile(path)) {
      throw new FileNotFoundError(`File ${path} not found`);
    }

    // Remove from ZIP
    const zipPath = path.startsWith('/') ? path.slice(1) : path;
    this.zip.remove(zipPath);

    // Remove from manifest
    this._manifest.files = this._manifest.files.filter(f => f.path !== path);

    // Remove from entrypoints if referenced
    for (const [entrypointName, entrypointPath] of Object.entries(
      this._manifest.entrypoints
    )) {
      if (entrypointPath === path) {
        delete this._manifest.entrypoints[entrypointName];
      }
    }

    // Update manifest in ZIP
    await this.updateManifestInZip();
  }

  // Entrypoint Modification Methods

  setEntrypoint(name: string, path: string): void {
    if (!this.hasFile(path)) {
      throw new FileNotFoundError(`Target file ${path} not found`);
    }

    this._manifest.entrypoints[name] = path;
  }

  removeEntrypoint(name: string): void {
    if (!this.hasEntrypoint(name)) {
      throw new BundleValidationError(`Entrypoint ${name} not found`);
    }

    delete this._manifest.entrypoints[name];
  }

  // Validation Methods

  validate(options: ValidationOptions = {}): ValidationResult {
    // Use the comprehensive validation system with enhanced error reporting
    const bundleSize = this.estimateBundleSize();
    return validateBundleComprehensive(
      this.zip,
      this._manifest,
      bundleSize,
      options
    );
  }

  isValid(options: ValidationOptions = {}): boolean {
    return this.validate(options).valid;
  }

  // Compression Methods

  isFileCompressed(path: string): boolean {
    const file = this.getFile(path);
    if (!file) {
      throw new FileNotFoundError(`File ${path} not found`);
    }
    return file.compressed || false;
  }

  getUncompressedSize(path: string): number | null {
    const file = this.getFile(path);
    if (!file) {
      throw new FileNotFoundError(`File ${path} not found`);
    }
    return file.uncompressedSize || null;
  }

  // Utility Methods

  getBundleInfo(): BundleInfo {
    const totalSize = this.estimateBundleSize();
    const compressedFiles = this._manifest.files.filter(
      f => f.compressed
    ).length;
    const uncompressedSize = this._manifest.files.reduce(
      (sum, file) => sum + (file.uncompressedSize || file.length),
      0
    );

    return {
      version: this._manifest.version,
      name: this._manifest.name,
      fileCount: this._manifest.files.length,
      totalSize,
      compressedFiles,
      entrypoints: Object.keys(this._manifest.entrypoints),
      uncompressedSize,
      createdAt: this._manifest.createdAt,
    };
  }

  estimateBundleSize(): number {
    // Rough estimation based on file sizes and ZIP overhead
    const fileSize = this._manifest.files.reduce(
      (sum, file) => sum + file.length,
      0
    );
    const manifestSize = JSON.stringify(this._manifest).length;
    const zipOverhead = Math.floor(fileSize * 0.1); // Rough estimate for ZIP headers

    return fileSize + manifestSize + zipOverhead;
  }

  async clone(): Promise<ZipBundle> {
    // Serialize and re-parse to create a deep copy
    const data = await this.toArrayBuffer();
    return (await ZipBundle.parse(data)) as ZipBundle;
  }

  async merge(
    other: Bundle,
    options: {
      conflictResolution?: 'error' | 'skip' | 'replace';
      entrypointConflictResolution?: 'error' | 'skip' | 'replace';
    } = {}
  ): Promise<ZipBundle> {
    const {
      conflictResolution = 'error',
      entrypointConflictResolution = 'error',
    } = options;

    // Create a new bundle starting from a copy of this one
    const merged = await this.clone();

    // Merge files
    for (const file of other.listFiles()) {
      const existingFile = merged.getFile(file.path);

      if (existingFile) {
        // Handle file conflict
        if (conflictResolution === 'error') {
          throw new BundleValidationError(
            `File conflict during merge: ${file.path} exists in both bundles`
          );
        } else if (conflictResolution === 'skip') {
          continue; // Skip this file
        }
        // If 'replace', continue to add the file
      }

      // Get file data from the other bundle
      const fileData = await other.getFileData(file.path);
      if (fileData) {
        await merged.addFile(file, fileData, { replace: true });
      }
    }

    // Merge entrypoints
    const otherEntrypoints = other.listEntrypoints();
    for (const [name, path] of Object.entries(otherEntrypoints)) {
      const existingPath = merged.getEntrypoint(name);

      if (existingPath) {
        // Handle entrypoint conflict
        if (entrypointConflictResolution === 'error') {
          throw new BundleValidationError(
            `Entrypoint conflict during merge: ${name} exists in both bundles`
          );
        } else if (entrypointConflictResolution === 'skip') {
          continue; // Skip this entrypoint
        }
        // If 'replace', continue to set the entrypoint
      }

      // Ensure the target file exists in the merged bundle
      if (merged.hasFile(path)) {
        merged.setEntrypoint(name, path);
      } else {
        // If the file doesn't exist (shouldn't happen with proper merge), skip the entrypoint
        console.warn(
          `Skipping entrypoint ${name} because target file ${path} doesn't exist`
        );
      }
    }

    return merged;
  }

  // Serialization Methods

  async toArrayBuffer(
    options: SerializationOptions = {}
  ): Promise<ArrayBuffer> {
    const { compressionLevel = 6, useZip64 = false, comment } = options;

    // Update manifest in ZIP before serializing
    await this.updateManifestInZip();

    try {
      return await this.zip.generateAsync({
        type: 'arraybuffer',
        compression: 'DEFLATE',
        compressionOptions: {
          level: compressionLevel,
        },
        platform: useZip64 ? 'UNIX' : 'DOS',
        comment,
      });
    } catch (error) {
      throw new BundleParseError(
        `Failed to serialize bundle: ${error instanceof Error ? error.message : 'Unknown error'}`
      );
    }
  }

  // Private helper methods

  private async updateManifestInZip(): Promise<void> {
    const manifestJson = JSON.stringify(this._manifest, null, 2);
    this.zip.file('manifest.json', manifestJson);
  }

  // Static Factory Methods

  static async createEmpty(
    options: { version?: number } = {}
  ): Promise<ZipBundle> {
    const { version = 1 } = options;

    const manifest: BundleManifestType = {
      version,
      createdAt: new Date().toISOString(),
      entrypoints: {},
      files: [],
    };

    const zip = new JSZip();

    // Add manifest to ZIP
    const manifestJson = JSON.stringify(manifest, null, 2);
    zip.file('manifest.json', manifestJson);

    return new ZipBundle(zip, manifest);
  }

  static async fromFiles(
    files: Map<string, ArrayBuffer>,
    options: { contentTypes?: Map<string, string> } = {}
  ): Promise<ZipBundle> {
    const bundle = await ZipBundle.createEmpty();
    const { contentTypes = new Map() } = options;

    for (const [path, data] of files) {
      const contentType = contentTypes.get(path) || 'application/octet-stream';

      const file: Omit<BundleFile, 'length'> = {
        path: path.startsWith('/') ? path : '/' + path,
        contentType,
        compressed: true,
        lastModified: new Date().toISOString(),
      };

      await bundle.addFile(file, data);
    }

    return bundle;
  }

  static async parse(
    data: ArrayBuffer,
    options: ParseOptions = {}
  ): Promise<ZipBundle> {
    const {
      strictValidation = true,
      validateFileReferences = true,
      maxSize,
    } = options;

    if (maxSize && data.byteLength > maxSize) {
      throw new BundleParseError(
        `Bundle size ${data.byteLength} exceeds maximum allowed size ${maxSize}`
      );
    }

    try {
      // Load ZIP
      const zip = await JSZip.loadAsync(data);

      // Extract and parse manifest
      const manifestFile = zip.file('manifest.json');
      if (!manifestFile) {
        throw new BundleParseError('No manifest.json found in bundle');
      }

      const manifestText = await manifestFile.async('text');
      let manifestData: unknown;

      try {
        manifestData = JSON.parse(manifestText);
      } catch (error) {
        throw new BundleParseError(
          `Invalid manifest JSON: ${error instanceof Error ? error.message : 'Unknown error'}`
        );
      }

      // Validate manifest with Zod
      let manifest: BundleManifestType;
      try {
        manifest = BundleManifestSchema.parse(manifestData);
      } catch (error) {
        if (strictValidation) {
          throw new BundleParseError(
            `Manifest validation failed: ${error instanceof Error ? error.message : 'Unknown error'}`
          );
        } else {
          // Fall back to basic structure validation
          manifest = manifestData as BundleManifestType;
        }
      }

      const bundle = new ZipBundle(zip, manifest, data);

      // Validate file references if requested
      if (validateFileReferences) {
        const validation = bundle.validate();
        if (!validation.valid && strictValidation) {
          throw new BundleValidationError(
            `Bundle validation failed: ${validation.errors.map(e => e.message).join(', ')}`
          );
        }
      }

      return bundle;
    } catch (error) {
      if (
        error instanceof BundleParseError ||
        error instanceof BundleValidationError
      ) {
        throw error;
      }
      throw new BundleParseError(
        `Failed to parse bundle: ${error instanceof Error ? error.message : 'Unknown error'}`
      );
    }
  }
}

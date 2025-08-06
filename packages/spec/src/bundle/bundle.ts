/**
 * Bundle class - Main interface for working with Bundle Package Files
 */

import type {
  BundleManifest,
  BundleFile,
  BundleInfo,
  AddFileOptions,
} from '../types/bundle.js';
import type {
  ValidationOptions,
  ValidationResult,
} from '../types/validation.js';

/**
 * The main Bundle class that represents a complete bundle package
 *
 * This class provides methods for:
 * - Reading and accessing bundle contents
 * - Modifying bundle structure (adding/removing files)
 * - Validating bundle integrity
 * - Serializing to binary format
 */
export abstract class Bundle {
  /**
   * Get the bundle manifest
   */
  abstract get manifest(): BundleManifest;

  /**
   * Get the raw bundle data (if available)
   */
  abstract get data(): ArrayBuffer | null;

  // File Access Methods

  /**
   * Get metadata for a specific file
   * @param path - Virtual path of the file within the bundle
   * @returns File metadata or null if not found
   */
  abstract getFile(path: string): BundleFile | null;

  /**
   * Get the binary data for a specific file
   * @param path - Virtual path of the file within the bundle
   * @returns File data as ArrayBuffer or null if not found
   */
  abstract getFileData(path: string): Promise<ArrayBuffer | null>;

  /**
   * Check if a file exists in the bundle
   * @param path - Virtual path of the file within the bundle
   * @returns True if the file exists
   */
  abstract hasFile(path: string): boolean;

  /**
   * Get a list of all files in the bundle
   * @returns Array of all bundle files
   */
  abstract listFiles(): BundleFile[];

  /**
   * Get the total number of files in the bundle
   * @returns Number of files
   */
  abstract getFileCount(): number;

  // Entrypoint Methods

  /**
   * Get the file path for a specific entrypoint
   * @param name - Name of the entrypoint
   * @returns File path or null if entrypoint not found
   */
  abstract getEntrypoint(name: string): string | null;

  /**
   * Check if an entrypoint exists
   * @param name - Name of the entrypoint
   * @returns True if the entrypoint exists
   */
  abstract hasEntrypoint(name: string): boolean;

  /**
   * Get all entrypoints as a name-to-path mapping
   * @returns Record mapping entrypoint names to file paths
   */
  abstract listEntrypoints(): Record<string, string>;

  /**
   * Get the names of all entrypoints
   * @returns Array of entrypoint names
   */
  abstract getEntrypointNames(): string[];

  // File Modification Methods

  /**
   * Add a new file to the bundle
   * @param file - File metadata
   * @param data - File content as ArrayBuffer
   * @param options - Additional options for adding the file
   * @throws {BundleValidationError} If the file path already exists and replace is false
   */
  abstract addFile(
    file: Omit<BundleFile, 'length'>,
    data: ArrayBuffer,
    options?: AddFileOptions
  ): Promise<void>;

  /**
   * Update an existing file's content
   * @param path - Virtual path of the file to update
   * @param data - New file content as ArrayBuffer
   * @param contentType - Optional new content type
   * @throws {FileNotFoundError} If the file doesn't exist
   */
  abstract updateFile(
    path: string,
    data: ArrayBuffer,
    contentType?: string
  ): Promise<void>;

  /**
   * Remove a file from the bundle
   * @param path - Virtual path of the file to remove
   * @throws {FileNotFoundError} If the file doesn't exist
   */
  abstract removeFile(path: string): Promise<void>;

  // Entrypoint Modification Methods

  /**
   * Set an entrypoint to point to a specific file
   * @param name - Name of the entrypoint
   * @param path - Virtual path of the target file
   * @throws {FileNotFoundError} If the target file doesn't exist
   */
  abstract setEntrypoint(name: string, path: string): void;

  /**
   * Remove an entrypoint
   * @param name - Name of the entrypoint to remove
   * @throws {EntrypointNotFoundError} If the entrypoint doesn't exist
   */
  abstract removeEntrypoint(name: string): void;

  // Validation Methods

  /**
   * Validate the bundle structure and integrity
   * @param options - Validation options
   * @returns Validation result with any errors or warnings
   */
  abstract validate(options?: ValidationOptions): ValidationResult;

  /**
   * Check if the bundle is valid
   * @param options - Validation options
   * @returns True if the bundle passes validation
   */
  abstract isValid(options?: ValidationOptions): boolean;

  // Compression Methods

  /**
   * Check if a specific file is compressed
   * @param path - Virtual path of the file
   * @returns True if the file is compressed
   * @throws {FileNotFoundError} If the file doesn't exist
   */
  abstract isFileCompressed(path: string): boolean;

  /**
   * Get the uncompressed size of a file (if compressed)
   * @param path - Virtual path of the file
   * @returns Uncompressed size in bytes, or null if not compressed or unknown
   * @throws {FileNotFoundError} If the file doesn't exist
   */
  abstract getUncompressedSize(path: string): number | null;

  // Utility Methods

  /**
   * Get summary information about the bundle
   * @returns Bundle information object
   */
  abstract getBundleInfo(): BundleInfo;

  /**
   * Estimate the total size of the bundle when serialized
   * @returns Estimated size in bytes
   */
  abstract estimateBundleSize(): number;

  /**
   * Create a deep copy of the bundle
   * @returns New bundle instance with the same content
   */
  abstract clone(): Promise<Bundle>;

  /**
   * Merge another bundle into this one
   * @param other - The bundle to merge
   * @param options - Merge options
   * @returns New bundle containing merged content
   */
  abstract merge(
    other: Bundle,
    options?: {
      /** How to handle file conflicts (default: 'error') */
      conflictResolution?: 'error' | 'skip' | 'replace';
      /** How to handle entrypoint conflicts (default: 'error') */
      entrypointConflictResolution?: 'error' | 'skip' | 'replace';
    }
  ): Promise<Bundle>;

  // Serialization Methods

  /**
   * Serialize the bundle to binary format
   * @returns ArrayBuffer containing the complete bundle
   */
  abstract toArrayBuffer(): Promise<ArrayBuffer>;

  /**
   * Get the bundle as a Buffer (Node.js only)
   * @returns Buffer containing the complete bundle
   */
  async toBuffer(): Promise<Buffer> {
    if (typeof Buffer === 'undefined') {
      throw new Error('Buffer is not available in this environment');
    }
    const arrayBuffer = await this.toArrayBuffer();
    return Buffer.from(arrayBuffer);
  }

  // Static Factory Methods

  /**
   * Create an empty bundle
   * @param options - Bundle creation options
   * @returns New empty bundle
   */
  static createEmpty(_options?: { version?: number }): Promise<Bundle> {
    throw new Error('createEmpty must be implemented by concrete Bundle class');
  }

  /**
   * Create a bundle from a collection of files
   * @param files - Map of file paths to file data
   * @param options - Bundle creation options
   * @returns New bundle containing the specified files
   *
   */
  static fromFiles(
    _files: Map<string, ArrayBuffer>,
    _options?: { contentTypes?: Map<string, string> }
  ): Promise<Bundle> {
    throw new Error('fromFiles must be implemented by concrete Bundle class');
  }

  /**
   * Parse a bundle from binary data
   * @param data - Binary bundle data
   * @returns Parsed bundle instance
   * @throws {BundleParseError} If the data cannot be parsed
   */
  static parse(_data: ArrayBuffer): Promise<Bundle> {
    throw new Error('parse must be implemented by concrete Bundle class');
  }

  /**
   * Parse a bundle from a Buffer (Node.js only)
   * @param buffer - Buffer containing bundle data
   * @returns Parsed bundle instance
   */
  static async fromBuffer(buffer: Buffer): Promise<Bundle> {
    // Create a proper ArrayBuffer from Buffer to avoid SharedArrayBuffer type issues
    const arrayBuffer = new ArrayBuffer(buffer.byteLength);
    const view = new Uint8Array(arrayBuffer);
    view.set(new Uint8Array(buffer));
    return await Bundle.parse(arrayBuffer);
  }
}

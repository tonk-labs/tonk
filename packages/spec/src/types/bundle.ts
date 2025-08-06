/**
 * Bundle Package File Format - Type Definitions
 *
 * This file contains TypeScript interfaces and types for the Bundle Package File Format,
 * which uses ZIP containers with a JSON manifest for packaging multiple files.
 */

/**
 * Version identifier for the bundle format
 */
export type BundleVersion = number;

/**
 * Mapping of entrypoint names to file paths within the bundle
 */
export type EntrypointMap = Record<string, string>;

/**
 * Metadata for a single file within the bundle
 */
export interface BundleFile {
  /** Virtual path of the file within the bundle (e.g., "/src/index.js") */
  path: string;

  /** Size of the file data in bytes */
  length: number;

  /** MIME type of the file content */
  contentType: string;

  /** Whether the file data is compressed within the ZIP */
  compressed?: boolean;

  /** Original uncompressed size (only present if compressed) */
  uncompressedSize?: number;

  /** Timestamp when the file was last modified (ISO 8601) */
  lastModified?: string;
}

/**
 * The main manifest structure that describes the bundle contents
 */
export interface BundleManifest {
  /** Format version identifier */
  version: BundleVersion;

  /** Human-readable name of the bundle */
  name?: string;

  /** Description of the bundle contents */
  description?: string;

  /** Timestamp when the bundle was created (ISO 8601) */
  createdAt?: string;

  /** Mapping of entrypoint names to file paths */
  entrypoints: EntrypointMap;

  /** Array of all files contained in the bundle */
  files: BundleFile[];

  /** Optional metadata for extensibility */
  metadata?: Record<string, unknown>;
}

/**
 * Summary information about a bundle
 */
export interface BundleInfo {
  /** Bundle format version */
  version: BundleVersion;

  /** Bundle name (if provided) */
  name?: string;

  /** Total number of files (excluding manifest.json) */
  fileCount: number;

  /** Total size of the ZIP archive in bytes */
  totalSize: number;

  /** Number of compressed files within the ZIP */
  compressedFiles: number;

  /** List of entrypoint names */
  entrypoints: string[];

  /** Estimated uncompressed size of all files */
  uncompressedSize: number;

  /** Creation timestamp from manifest */
  createdAt?: string;
}

/**
 * Options for creating a new bundle
 */
export interface CreateBundleOptions {
  /** Bundle format version (defaults to latest) */
  version?: BundleVersion;

  /** Human-readable name */
  name?: string;

  /** Description of the bundle */
  description?: string;

  /** Custom metadata */
  metadata?: Record<string, unknown>;
}

/**
 * Options for adding files to a bundle
 */
export interface AddFileOptions {
  /** MIME type override (auto-detected if not provided) */
  contentType?: string;

  /** Whether to compress the file data within the ZIP (default: true) */
  compress?: boolean;

  /** Replace existing file if it exists (default: false) */
  replace?: boolean;

  /** Custom last modified timestamp (ISO 8601) */
  lastModified?: string;
}

/**
 * Options for serializing a bundle to ArrayBuffer
 */
export interface SerializationOptions {
  /** Compression level for ZIP (0-9, default: 6) */
  compressionLevel?: number;

  /** Whether to use ZIP64 format for large bundles */
  useZip64?: boolean;

  /** Comment to include in the ZIP file */
  comment?: string;
}

/**
 * Options for parsing a bundle from ArrayBuffer
 */
export interface ParseOptions {
  /** Whether to validate the manifest schema strictly */
  strictValidation?: boolean;

  /** Whether to check that all manifest files exist in ZIP */
  validateFileReferences?: boolean;

  /** Maximum bundle size to accept (in bytes) */
  maxSize?: number;
}

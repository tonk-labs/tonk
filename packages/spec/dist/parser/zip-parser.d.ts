import { default as JSZip } from 'jszip';
import { ZipBundle } from '../bundle/zip-bundle.js';
import { BundleManifestType } from '../schemas/bundle.js';
import { ParseOptions } from '../types/bundle.js';

/**
 * Parse a bundle from an ArrayBuffer containing ZIP data
 *
 * This is the main entry point for parsing ZIP-based bundles. It:
 * 1. Loads the ZIP archive using JSZip
 * 2. Extracts and validates the manifest.json
 * 3. Creates a ZipBundle instance with validated data
 * 4. Optionally validates file references match ZIP contents
 *
 * @param buffer - ArrayBuffer containing the ZIP-based bundle
 * @param options - Parsing options for validation and size limits
 * @returns Promise that resolves to a ZipBundle instance
 * @throws BundleParseError for parsing failures
 * @throws BundleValidationError for validation failures
 */
export declare function parseBundle(buffer: ArrayBuffer, options?: ParseOptions): Promise<ZipBundle>;
/**
 * Extract and validate the manifest.json from a JSZip instance
 *
 * @param zip - JSZip instance to extract manifest from
 * @param strictValidation - Whether to enforce strict Zod validation
 * @returns Promise that resolves to validated manifest data
 * @throws BundleParseError if manifest is missing or invalid
 */
export declare function extractManifest(zip: JSZip, strictValidation?: boolean): Promise<BundleManifestType>;
/**
 * Build internal file maps from ZIP contents
 *
 * This function creates a mapping between the ZIP file entries and the
 * manifest file declarations to ensure consistency.
 *
 * @param zip - JSZip instance containing bundle files
 * @param manifest - Validated bundle manifest
 * @returns Map of virtual paths to ZIP file objects
 */
export declare function buildFileMap(zip: JSZip, manifest: BundleManifestType): Map<string, JSZip.JSZipObject>;
/**
 * Validate that manifest file references match ZIP contents
 *
 * This function ensures that all files declared in the manifest
 * actually exist in the ZIP archive.
 *
 * @param zip - JSZip instance containing bundle files
 * @param manifest - Validated bundle manifest
 * @returns Array of validation errors (empty if all files exist)
 */
export declare function validateFileReferences(zip: JSZip, manifest: BundleManifestType): string[];
/**
 * Create a ZipBundle from pre-loaded JSZip and manifest data
 *
 * This is a lower-level function for creating ZipBundle instances when
 * you already have validated ZIP and manifest data.
 *
 * @param zip - JSZip instance containing bundle data
 * @param manifest - Validated bundle manifest
 * @param sourceData - Original source data (optional)
 * @returns ZipBundle instance
 */
export declare function createBundleFromZip(zip: JSZip, manifest: BundleManifestType, sourceData?: ArrayBuffer): ZipBundle;
/**
 * Convenience function to check if an ArrayBuffer contains valid ZIP data
 *
 * @param buffer - ArrayBuffer to check
 * @returns Promise that resolves to true if valid ZIP, false otherwise
 */
export declare function isValidZip(buffer: ArrayBuffer): Promise<boolean>;
//# sourceMappingURL=zip-parser.d.ts.map
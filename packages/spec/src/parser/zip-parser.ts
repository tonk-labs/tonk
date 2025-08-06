/**
 * ZIP-based Bundle Parser
 *
 * This module provides utilities for parsing ZIP-based bundle files using JSZip.
 * It implements the phase 3 requirements from the implementation plan.
 */

import JSZip from 'jszip';
import { ZipBundle } from '../bundle/zip-bundle.js';
import {
  BundleManifestSchema,
  type BundleManifestType,
} from '../schemas/bundle.js';
import type { ParseOptions } from '../types/bundle.js';
import { BundleParseError } from '../types/errors.js';

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
export async function parseBundle(
  buffer: ArrayBuffer,
  options: ParseOptions = {}
): Promise<ZipBundle> {
  return ZipBundle.parse(buffer, options);
}

/**
 * Extract and validate the manifest.json from a JSZip instance
 *
 * @param zip - JSZip instance to extract manifest from
 * @param strictValidation - Whether to enforce strict Zod validation
 * @returns Promise that resolves to validated manifest data
 * @throws BundleParseError if manifest is missing or invalid
 */
export async function extractManifest(
  zip: JSZip,
  strictValidation = true
): Promise<BundleManifestType> {
  // Extract manifest.json from ZIP archive
  const manifestFile = zip.file('manifest.json');
  if (!manifestFile) {
    throw BundleParseError.missingManifest();
  }

  // Parse JSON content
  let manifestText: string;
  try {
    manifestText = await manifestFile.async('text');
  } catch (error) {
    throw new BundleParseError(
      `Failed to read manifest.json: ${error instanceof Error ? error.message : 'Unknown error'}`
    );
  }

  let manifestData: unknown;
  try {
    manifestData = JSON.parse(manifestText);
  } catch (error) {
    throw BundleParseError.invalidManifestJson(error as Error);
  }

  // Validate manifest using Zod schemas
  try {
    return BundleManifestSchema.parse(manifestData);
  } catch (error) {
    if (strictValidation) {
      throw new BundleParseError(
        `Manifest validation failed: ${error instanceof Error ? error.message : 'Unknown error'}`
      );
    } else {
      // Fall back to basic structure validation in non-strict mode
      return manifestData as BundleManifestType;
    }
  }
}

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
export function buildFileMap(
  zip: JSZip,
  manifest: BundleManifestType
): Map<string, JSZip.JSZipObject> {
  const fileMap = new Map<string, JSZip.JSZipObject>();

  for (const manifestFile of manifest.files) {
    // Convert virtual path to ZIP path (remove leading /)
    const zipPath = manifestFile.path.startsWith('/')
      ? manifestFile.path.slice(1)
      : manifestFile.path;

    const zipFile = zip.file(zipPath);
    if (zipFile) {
      fileMap.set(manifestFile.path, zipFile);
    }
  }

  return fileMap;
}

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
export function validateFileReferences(
  zip: JSZip,
  manifest: BundleManifestType
): string[] {
  const errors: string[] = [];

  for (const manifestFile of manifest.files) {
    // Convert virtual path to ZIP path (remove leading /)
    const zipPath = manifestFile.path.startsWith('/')
      ? manifestFile.path.slice(1)
      : manifestFile.path;

    const zipFile = zip.file(zipPath);
    if (!zipFile) {
      errors.push(
        `File referenced in manifest not found in ZIP: ${manifestFile.path}`
      );
    }
  }

  // Also check entrypoints reference existing files
  if (manifest.entrypoints) {
    for (const [entrypointName, filePath] of Object.entries(
      manifest.entrypoints
    )) {
      const fileExists = manifest.files.some(file => file.path === filePath);
      if (!fileExists) {
        errors.push(
          `Entrypoint '${entrypointName}' references non-existent file: ${filePath}`
        );
      }
    }
  }

  return errors;
}

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
export function createBundleFromZip(
  zip: JSZip,
  manifest: BundleManifestType,
  sourceData?: ArrayBuffer
): ZipBundle {
  return new ZipBundle(zip, manifest, sourceData);
}

/**
 * Convenience function to check if an ArrayBuffer contains valid ZIP data
 *
 * @param buffer - ArrayBuffer to check
 * @returns Promise that resolves to true if valid ZIP, false otherwise
 */
export async function isValidZip(buffer: ArrayBuffer): Promise<boolean> {
  try {
    await JSZip.loadAsync(buffer);
    return true;
  } catch {
    return false;
  }
}

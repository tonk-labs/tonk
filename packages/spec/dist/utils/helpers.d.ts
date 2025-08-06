import { Bundle } from '../bundle/bundle.js';
import { BundleInfo } from '../types/bundle.js';

/**
 * Create an empty bundle with default settings
 *
 * @param options - Bundle creation options
 * @returns A new empty Bundle instance
 *
 * @example
 * ```typescript
 * const bundle = await createEmptyBundle();
 * // or with specific version
 * const bundleV2 = await createEmptyBundle({ version: 2 });
 * ```
 */
export declare function createEmptyBundle(options?: {
    version?: number;
}): Promise<Bundle>;
/**
 * Get summary information about a bundle
 *
 * This is a convenience function that delegates to the bundle's
 * getBundleInfo() method.
 *
 * @param bundle - The bundle to get info from
 * @returns Bundle information summary
 *
 * @example
 * ```typescript
 * const info = getBundleInfo(bundle);
 * console.log(`Bundle has ${info.fileCount} files`);
 * console.log(`Total size: ${info.totalSize} bytes`);
 * ```
 */
export declare function getBundleInfo(bundle: Bundle): BundleInfo;
/**
 * Estimate the total size of a bundle when serialized
 *
 * This is a convenience function that delegates to the bundle's
 * estimateBundleSize() method.
 *
 * @param bundle - The bundle to estimate size for
 * @returns Estimated size in bytes
 *
 * @example
 * ```typescript
 * const size = await estimateBundleSize(bundle);
 * console.log(`Bundle will be approximately ${size} bytes`);
 * ```
 */
export declare function estimateBundleSize(bundle: Bundle): Promise<number>;
/**
 * Guess the MIME type based on file extension
 *
 * @param filename - The filename or path to check
 * @returns The guessed MIME type, or 'application/octet-stream' as fallback
 *
 * @example
 * ```typescript
 * guessMimeType('index.html'); // 'text/html'
 * guessMimeType('/assets/style.css'); // 'text/css'
 * guessMimeType('data.bin'); // 'application/octet-stream'
 * ```
 */
export declare function guessMimeType(filename: string): string;
/**
 * Format byte size to human readable format
 *
 * @param bytes - Number of bytes
 * @param decimals - Number of decimal places (default: 2)
 * @returns Formatted string with unit
 *
 * @example
 * ```typescript
 * formatBytes(1024); // '1.00 KB'
 * formatBytes(1234567); // '1.18 MB'
 * formatBytes(1234567890); // '1.15 GB'
 * ```
 */
export declare function formatBytes(bytes: number, decimals?: number): string;
/**
 * Create a bundle from a file map with automatic MIME type detection
 *
 * @param files - Map of file paths to file data
 * @param options - Additional options
 * @returns A new Bundle containing the files
 *
 * @example
 * ```typescript
 * const files = new Map([
 *   ['/index.html', htmlBuffer],
 *   ['/style.css', cssBuffer],
 *   ['/app.js', jsBuffer],
 * ]);
 *
 * const bundle = await createBundleFromFiles(files);
 * ```
 */
export declare function createBundleFromFiles(files: Map<string, ArrayBuffer>, options?: {
    contentTypes?: Map<string, string>;
    autoDetectTypes?: boolean;
}): Promise<Bundle>;
/**
 * Merge multiple bundles into a single bundle
 *
 * @param bundles - Array of bundles to merge
 * @param options - Merge options
 * @returns A new bundle containing all merged content
 *
 * @example
 * ```typescript
 * // Merge with error on conflicts (default)
 * const merged = await mergeBundles([bundle1, bundle2, bundle3]);
 *
 * // Merge with replace strategy for conflicts
 * const merged = await mergeBundles([bundle1, bundle2], {
 *   conflictResolution: 'replace',
 *   entrypointConflictResolution: 'replace'
 * });
 *
 * // Merge with skip strategy for file conflicts
 * const merged = await mergeBundles([bundle1, bundle2], {
 *   conflictResolution: 'skip'
 * });
 * ```
 */
export declare function mergeBundles(bundles: Bundle[], options?: {
    conflictResolution?: 'error' | 'skip' | 'replace';
    entrypointConflictResolution?: 'error' | 'skip' | 'replace';
}): Promise<Bundle>;
//# sourceMappingURL=helpers.d.ts.map
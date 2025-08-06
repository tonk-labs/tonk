/**
 * General helper functions for Bundle operations
 */

import { ZipBundle } from '../bundle/zip-bundle.js';
import type { Bundle } from '../bundle/bundle.js';
import type { BundleInfo } from '../types/bundle.js';

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
export async function createEmptyBundle(
  options: { version?: number } = {}
): Promise<Bundle> {
  return await ZipBundle.createEmpty(options);
}

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
export function getBundleInfo(bundle: Bundle): BundleInfo {
  return bundle.getBundleInfo();
}

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
export async function estimateBundleSize(bundle: Bundle): Promise<number> {
  // Some implementations might need async operations
  return bundle.estimateBundleSize();
}

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
export function guessMimeType(filename: string): string {
  const ext = filename.toLowerCase().split('.').pop();

  const mimeTypes: Record<string, string> = {
    // Text
    html: 'text/html',
    htm: 'text/html',
    css: 'text/css',
    js: 'application/javascript',
    mjs: 'application/javascript',
    json: 'application/json',
    txt: 'text/plain',
    xml: 'application/xml',

    // Images
    png: 'image/png',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    gif: 'image/gif',
    svg: 'image/svg+xml',
    webp: 'image/webp',
    ico: 'image/x-icon',

    // Fonts
    woff: 'font/woff',
    woff2: 'font/woff2',
    ttf: 'font/ttf',
    otf: 'font/otf',

    // Documents
    pdf: 'application/pdf',

    // Audio/Video
    mp3: 'audio/mpeg',
    mp4: 'video/mp4',
    webm: 'video/webm',
    ogg: 'audio/ogg',

    // Archives
    zip: 'application/zip',
    tar: 'application/x-tar',
    gz: 'application/gzip',

    // Programming languages
    ts: 'text/typescript',
    tsx: 'text/typescript',
    jsx: 'text/javascript',
    py: 'text/x-python',
    rs: 'text/x-rust',
    go: 'text/x-go',
    java: 'text/x-java',
    c: 'text/x-c',
    cpp: 'text/x-c++',
    h: 'text/x-c',
    hpp: 'text/x-c++',
    cs: 'text/x-csharp',
    php: 'text/x-php',
    rb: 'text/x-ruby',
    swift: 'text/x-swift',
    kt: 'text/x-kotlin',

    // Web Assembly
    wasm: 'application/wasm',
  };

  return mimeTypes[ext || ''] || 'application/octet-stream';
}

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
export function formatBytes(bytes: number, decimals = 2): string {
  if (bytes === 0) return '0 Bytes';

  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];

  const i = Math.floor(Math.log(bytes) / Math.log(k));

  return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
}

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
export async function createBundleFromFiles(
  files: Map<string, ArrayBuffer>,
  options: {
    contentTypes?: Map<string, string>;
    autoDetectTypes?: boolean;
  } = {}
): Promise<Bundle> {
  const { contentTypes = new Map(), autoDetectTypes = true } = options;

  // Auto-detect content types if enabled
  if (autoDetectTypes) {
    for (const [path] of files) {
      if (!contentTypes.has(path)) {
        contentTypes.set(path, guessMimeType(path));
      }
    }
  }

  return await ZipBundle.fromFiles(files, { contentTypes });
}

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
export async function mergeBundles(
  bundles: Bundle[],
  options: {
    conflictResolution?: 'error' | 'skip' | 'replace';
    entrypointConflictResolution?: 'error' | 'skip' | 'replace';
  } = {}
): Promise<Bundle> {
  if (bundles.length === 0) {
    throw new Error('No bundles provided to merge');
  }

  if (bundles.length === 1) {
    return bundles[0].clone();
  }

  // Start with the first bundle as base
  let result = await bundles[0].clone();

  // Merge each subsequent bundle
  for (let i = 1; i < bundles.length; i++) {
    result = await result.merge(bundles[i], options);
  }

  return result;
}

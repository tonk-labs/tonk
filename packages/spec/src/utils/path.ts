/**
 * Path validation utilities for Bundle virtual paths
 */

import { z } from 'zod';

/**
 * Zod schema for validating virtual file paths
 *
 * Valid paths must:
 * - Start with a forward slash
 * - Only contain alphanumeric characters, dots, dashes, underscores, and forward slashes
 * - Not contain empty segments (no double slashes)
 * - Not end with a slash (no directories)
 * - Not contain relative path components (. or ..)
 */
export const VirtualPathSchema = z
  .string()
  .min(2, 'Path must be at least 2 characters')
  .regex(/^\/[a-zA-Z0-9._-]+(\/[a-zA-Z0-9._-]+)*$/, {
    message: 'Invalid virtual path format',
  })
  .refine(path => !path.includes('//'), {
    message: 'Path cannot contain empty segments (//)',
  })
  .refine(path => !path.includes('/.'), {
    message: 'Path cannot contain relative path components (. or ..)',
  });

/**
 * Validate a virtual file path
 *
 * @param path - The path to validate
 * @returns True if the path is valid, false otherwise
 *
 * @example
 * ```typescript
 * validatePath('/index.html'); // true
 * validatePath('/assets/images/logo.png'); // true
 * validatePath('index.html'); // false - must start with /
 * validatePath('/assets/'); // false - cannot end with /
 * validatePath('/assets/../index.html'); // false - no relative paths
 * ```
 */
export function validatePath(path: string): boolean {
  try {
    VirtualPathSchema.parse(path);
    return true;
  } catch {
    return false;
  }
}

/**
 * Normalize a path to ensure it starts with a forward slash
 *
 * @param path - The path to normalize
 * @returns The normalized path
 *
 * @example
 * ```typescript
 * normalizePath('index.html'); // '/index.html'
 * normalizePath('/index.html'); // '/index.html'
 * ```
 */
export function normalizePath(path: string): string {
  if (!path.startsWith('/')) {
    return '/' + path;
  }
  return path;
}

/**
 * Get the directory name from a path
 *
 * @param path - The path to extract the directory from
 * @returns The directory path, or '/' if the file is in the root
 *
 * @example
 * ```typescript
 * dirname('/assets/images/logo.png'); // '/assets/images'
 * dirname('/index.html'); // '/'
 * ```
 */
export function dirname(path: string): string {
  const lastSlash = path.lastIndexOf('/');
  if (lastSlash <= 0) {
    return '/';
  }
  return path.substring(0, lastSlash);
}

/**
 * Get the base name (filename) from a path
 *
 * @param path - The path to extract the filename from
 * @returns The filename
 *
 * @example
 * ```typescript
 * basename('/assets/images/logo.png'); // 'logo.png'
 * basename('/index.html'); // 'index.html'
 * ```
 */
export function basename(path: string): string {
  const lastSlash = path.lastIndexOf('/');
  return path.substring(lastSlash + 1);
}

/**
 * Get the file extension from a path
 *
 * @param path - The path to extract the extension from
 * @returns The file extension (including the dot), or empty string if no extension
 *
 * @example
 * ```typescript
 * extname('/assets/images/logo.png'); // '.png'
 * extname('/index.html'); // '.html'
 * extname('/README'); // ''
 * ```
 */
export function extname(path: string): string {
  const filename = basename(path);
  const lastDot = filename.lastIndexOf('.');
  if (lastDot <= 0) {
    return '';
  }
  return filename.substring(lastDot);
}

/**
 * Join path segments together
 *
 * @param segments - Path segments to join
 * @returns The joined path
 *
 * @example
 * ```typescript
 * join('/assets', 'images', 'logo.png'); // '/assets/images/logo.png'
 * join('/', 'index.html'); // '/index.html'
 * ```
 */
export function join(...segments: string[]): string {
  const parts: string[] = [];

  for (const segment of segments) {
    if (segment === '/') {
      if (parts.length === 0) {
        parts.push('');
      }
    } else if (segment) {
      // Remove leading and trailing slashes from segment
      const cleanSegment = segment.replace(/^\/+|\/+$/g, '');
      if (cleanSegment) {
        parts.push(cleanSegment);
      }
    }
  }

  // Ensure path starts with /
  if (parts.length === 0 || parts[0] !== '') {
    parts.unshift('');
  }

  return parts.join('/') || '/';
}

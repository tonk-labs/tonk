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
export declare const VirtualPathSchema: z.ZodString;
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
export declare function validatePath(path: string): boolean;
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
export declare function normalizePath(path: string): string;
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
export declare function dirname(path: string): string;
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
export declare function basename(path: string): string;
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
export declare function extname(path: string): string;
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
export declare function join(...segments: string[]): string;
//# sourceMappingURL=path.d.ts.map
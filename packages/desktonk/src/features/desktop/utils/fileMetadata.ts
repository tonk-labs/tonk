import type { DocumentData } from '@tonk/core';
import type { DesktopFile } from '../types';
import { getMimeType } from './mimeResolver';

/**
 * Grid layout constants for auto-positioning file icons.
 */
export const GRID_SPACING = 120; // pixels between icon centers
export const GRID_COLUMNS = 8; // number of icons per row before wrapping
export const GRID_START_OFFSET = 50; // pixels from top-left corner to first icon

/**
 * Validates and safely extracts a number from metadata.
 */
function safeNumber(value: unknown): number | undefined {
  if (typeof value === 'number' && !Number.isNaN(value) && Number.isFinite(value)) {
    return value;
  }
  return undefined;
}

/**
 * Validates and safely extracts a string from metadata.
 */
function safeString(value: unknown): string | undefined {
  if (typeof value === 'string' && value.length > 0) {
    return value;
  }
  return undefined;
}

/**
 * Extracts and validates desktop file metadata from a VFS document.
 * Safely handles malformed or missing metadata by using defaults.
 *
 * @param path - Full VFS path to the file (e.g., "/desktonk/notes.txt")
 * @param doc - DocumentData from VFS containing content and metadata
 * @returns DesktopFile with validated metadata or sensible defaults
 *
 * @example
 * ```typescript
 * const doc = await vfs.readFile('/desktonk/notes.txt');
 * const file = extractDesktopFile('/desktonk/notes.txt', doc);
 * // file.desktopMeta?.x and file.desktopMeta?.y contain icon position
 * ```
 */
export function extractDesktopFile(path: string, doc: DocumentData): DesktopFile {
  const content = doc.content;

  // Validate content is an object
  if (!content || typeof content !== 'object') {
    console.warn(`[fileMetadata] Invalid content for ${path}, using defaults`);
    return {
      path,
      name: doc.name,
      mimeType: getMimeType(doc.name),
    };
  }

  const contentObj = content as Record<string, unknown>;
  const desktopMeta = contentObj.desktopMeta;

  // Validate desktopMeta is an object if it exists
  let validatedMeta: Record<string, unknown> | undefined;
  if (desktopMeta && typeof desktopMeta === 'object') {
    validatedMeta = desktopMeta as Record<string, unknown>;
  }

  // Detect MIME type from filename if not in metadata
  const detectedMime = getMimeType(doc.name);
  const mimeType = safeString(validatedMeta?.mimeType) || detectedMime;

  return {
    path,
    name: doc.name,
    mimeType,
    desktopMeta: validatedMeta
      ? {
          x: safeNumber(validatedMeta.x),
          y: safeNumber(validatedMeta.y),
          icon: safeString(validatedMeta.icon),
          appHandler: safeString(validatedMeta.appHandler),
          thumbnail: safeString(validatedMeta.thumbnail),
        }
      : undefined,
  };
}

/**
 * Calculates the auto-layout position for a file icon based on its index.
 * Arranges icons in a grid pattern with configurable spacing and columns.
 *
 * @param index - Zero-based index of the file icon in the list
 * @returns Object with x and y coordinates in pixels
 *
 * @example
 * ```typescript
 * const pos0 = getNextAutoLayoutPosition(0);  // { x: 50, y: 50 } (first icon)
 * const pos1 = getNextAutoLayoutPosition(1);  // { x: 170, y: 50 } (second icon)
 * const pos8 = getNextAutoLayoutPosition(8);  // { x: 50, y: 170 } (first icon, second row)
 * ```
 */
export function getNextAutoLayoutPosition(index: number): { x: number; y: number } {
  const col = index % GRID_COLUMNS;
  const row = Math.floor(index / GRID_COLUMNS);

  return {
    x: GRID_START_OFFSET + col * GRID_SPACING,
    y: GRID_START_OFFSET + row * GRID_SPACING,
  };
}

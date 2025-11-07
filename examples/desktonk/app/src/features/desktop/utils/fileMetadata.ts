import type { DocumentData } from '@tonk/core';
import type { DesktopFile } from '../types';
import { getMimeType } from './mimeResolver';

/**
 * Validates and safely extracts a number from metadata.
 */
function safeNumber(value: unknown): number | undefined {
  if (typeof value === 'number' && !isNaN(value) && isFinite(value)) {
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

export function extractDesktopFile(path: string, doc: DocumentData): DesktopFile {
  const content = doc.content;

  // Validate content is an object
  if (!content || typeof content !== 'object') {
    console.warn(`Invalid content for ${path}, using defaults`);
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
        }
      : undefined,
  };
}

export function getNextAutoLayoutPosition(index: number): { x: number; y: number } {
  const gridSize = 120;
  const columns = 8;

  const col = index % columns;
  const row = Math.floor(index / columns);

  return {
    x: 50 + (col * gridSize),
    y: 50 + (row * gridSize),
  };
}

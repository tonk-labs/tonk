import type { DocumentData } from '@tonk/core';
import type { DesktopFile } from '../types';
import { getMimeType } from './mimeResolver';

export function extractDesktopFile(path: string, doc: DocumentData): DesktopFile {
  const content = doc.content as any;
  const desktopMeta = content?.desktopMeta;

  // Detect MIME type from filename if not in metadata
  const detectedMime = getMimeType(doc.name);
  const mimeType = desktopMeta?.mimeType || detectedMime;

  return {
    path,
    name: doc.name,
    mimeType,
    desktopMeta: {
      x: desktopMeta?.x,
      y: desktopMeta?.y,
      icon: desktopMeta?.icon,
      appHandler: desktopMeta?.appHandler,
    },
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

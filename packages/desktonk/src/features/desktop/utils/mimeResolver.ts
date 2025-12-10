import mime from 'mime';

export function getMimeType(fileName: string): string {
  const detected = mime.getType(fileName);
  return detected || 'application/octet-stream';
}

export function getFileIcon(mimeType: string): string {
  if (mimeType.startsWith('text/')) return '📄';
  if (mimeType.startsWith('image/')) return '🖼';
  if (mimeType === 'application/json') return '📋';
  if (mimeType === 'application/pdf') return '📕';
  if (mimeType.startsWith('video/')) return '🎬';
  if (mimeType.startsWith('audio/')) return '🎵';
  return '📦';
}

export const MIME_TO_APP: Record<string, string> = {
  'text/plain': 'text-editor',
  'text/markdown': 'text-editor',
  'application/json': 'text-editor',
  'text/html': 'text-editor',
  // Future apps can be added here
};

export function getAppHandler(mimeType: string, override?: string): string {
  return override || MIME_TO_APP[mimeType] || 'text-editor';
}

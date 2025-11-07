/**
 * Represents a file on the desktop with its metadata.
 */
export interface DesktopFile {
  path: string;
  name: string;
  mimeType: string;
  desktopMeta?: {
    x?: number;
    y?: number;
    icon?: string;
    appHandler?: string;
    thumbnail?: string;
  };
}

/**
 * Props for the FileIcon shape in TLDraw.
 * Defines the properties needed to render and interact with file icons on the desktop.
 */
export interface FileIconShapeProps {
  filePath: string;
  fileName: string;
  mimeType: string;
  customIcon?: string;
  thumbnail?: string;
  appHandler?: string;
  w: number;
  h: number;
}

export interface DesktopFile {
  path: string;
  name: string;
  mimeType: string;
  desktopMeta?: {
    x?: number;
    y?: number;
    icon?: string;
    appHandler?: string;
  };
}

export interface FileIconShapeProps {
  filePath: string;
  fileName: string;
  mimeType: string;
  customIcon?: string;
  appHandler?: string;
  w: number;
  h: number;
}

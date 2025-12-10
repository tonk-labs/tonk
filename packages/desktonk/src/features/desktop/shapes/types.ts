import type { TLBaseShape } from 'tldraw';

export type FileIconShape = TLBaseShape<
  'file-icon',
  {
    filePath: string;
    fileName: string;
    mimeType: string;
    customIcon?: string;
    /** VFS path to the thumbnail image file (e.g., /var/lib/desktonk/thumbnails/myfile.png) */
    thumbnailPath?: string;
    appHandler?: string;
    w: number;
    h: number;
  }
>;

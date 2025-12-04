import type { TLBaseShape } from 'tldraw';

export type FileIconShape = TLBaseShape<
  'file-icon',
  {
    filePath: string;
    fileName: string;
    mimeType: string;
    customIcon?: string;
    thumbnail?: string;
    appHandler?: string;
    w: number;
    h: number;
  }
>;

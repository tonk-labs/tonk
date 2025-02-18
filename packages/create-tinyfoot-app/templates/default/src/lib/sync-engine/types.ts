export type DocumentId = string;
export type BlobId = string;

export interface SyncEngineOptions {
  websocketUrl: string;
  dbName?: string;
  onSync?: (docId: DocumentId) => void;
  onError?: (error: Error) => void;
}

export interface Document {
  content: any;
  blobs: Record<BlobId, Blob>;
}

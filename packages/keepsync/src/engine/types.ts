export type DocumentId = string;
export type BlobId = string;

// Enhanced SyncEngine options
export interface SyncEngineOptions {
  url?: string;
  dbName?: string;
  name?: string;
  onSync?: (docId: DocumentId) => void;
  onError?: (error: Error) => void;
}

// Document-related types
export interface DocumentData {
  [key: string]: any;
}

// Connection-related types
export interface ConnectionOptions {
  url: string;
  reconnectDelay?: number;
}

// Message-related types
export type MessageHandler = (data: any) => Promise<void>;

export interface SyncMessage {
  docId: DocumentId;
  changes: number[];
}

export interface CustomMessage {
  type: string;
  [key: string]: any;
}

export interface ClientJoinedMessage extends CustomMessage {
  type: 'client_joined';
  clientId: string;
  timestamp: number;
}

// Storage-related types
export interface StorageProvider {
  init(): Promise<void>;
  saveDocument(id: DocumentId, doc: Uint8Array): Promise<void>;
  getDocument(id: DocumentId): Promise<Uint8Array | null>;
  saveBlob?(id: BlobId, blob: Blob): Promise<void>;
  getBlob?(id: BlobId): Promise<Blob | null>;
}

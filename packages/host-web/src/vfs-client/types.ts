// Re-export types from service-worker for client use
export type {
  DocumentContent,
  VFSWorkerMessage,
  VFSWorkerResponse,
  JsonValue,
  DocumentData,
  RefNode,
  Manifest,
} from '../service-worker/types';

// Client-specific types
export type ConnectionState =
  | 'disconnected'
  | 'connecting'
  | 'open'
  | 'connected'
  | 'reconnecting';

export type ConnectionStateListener = (state: ConnectionState) => void;

export interface WatcherMetadata {
  path: string;
  type: 'file' | 'directory';
}

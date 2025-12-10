// VFS Client - Client-side VFS service for communicating with the service worker

// Export types
export type {
  DocumentContent,
  VFSWorkerMessage,
  VFSWorkerResponse,
  JsonValue,
  DocumentData,
  RefNode,
  Manifest,
  ConnectionState,
  ConnectionStateListener,
  WatcherMetadata,
} from './types';

// Export VFS service
export { VFSService, getVFSService, resetVFSService } from './vfs-service';

// Export utilities
export {
  bytesToString,
  stringToBytes,
  uint8ArrayToBase64,
  base64ToUint8Array,
} from './vfs-utils';

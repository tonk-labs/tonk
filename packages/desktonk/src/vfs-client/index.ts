// VFS Client - Client-side VFS service for communicating with the service worker

// Export types
export type {
  ConnectionState,
  ConnectionStateListener,
  DocumentContent,
  DocumentData,
  JsonValue,
  Manifest,
  RefNode,
  VFSWorkerMessage,
  VFSWorkerResponse,
  WatcherMetadata,
} from './types';

// Export VFS service
export { getVFSService, resetVFSService, VFSService } from './vfs-service';

// Export utilities
export {
  base64ToUint8Array,
  bytesToString,
  stringToBytes,
  uint8ArrayToBase64,
} from './vfs-utils';

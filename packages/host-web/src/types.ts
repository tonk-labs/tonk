// Re-export all types from the consolidated service-worker types
// This maintains backward compatibility for any code importing from './types'
export type {
  DocumentContent,
  TonkState,
  WatcherHandle,
  FetchEvent,
  VFSWorkerMessage,
  VFSWorkerResponse,
  JsonValue,
  DocumentData,
  RefNode,
  Manifest,
} from './service-worker/types';

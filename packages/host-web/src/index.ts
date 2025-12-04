// @tonk/host-web - Browser runtime for Tonk applications
//
// This package provides:
// - Service worker for VFS operations and HTTP interception
// - VFS client for communicating with the service worker from apps

// Re-export types from the consolidated types file
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
} from './types';

// Note: For the VFS client, import from '@tonk/host-web/client'
// For service worker internals, import from '@tonk/host-web/service-worker'

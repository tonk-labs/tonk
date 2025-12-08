import type { JsonValue, DocumentData, RefNode, Manifest } from '@tonk/core/slim';

// Re-export core types for convenience
export type { JsonValue, DocumentData, RefNode, Manifest };

// Document content for write operations
export type DocumentContent = {
  content: JsonValue;
  bytes?: string;
};

// Message types for VFS Worker communication (requests from client)
export type VFSWorkerMessage =
  | { type: 'init'; manifest: ArrayBuffer; wsUrl: string }
  | { type: 'readFile'; id: string; path: string }
  | {
      type: 'writeFile';
      id: string;
      path: string;
      content: DocumentContent;
      create: boolean;
    }
  | { type: 'deleteFile'; id: string; path: string }
  | { type: 'rename'; id: string; oldPath: string; newPath: string }
  | { type: 'listDirectory'; id: string; path: string }
  | { type: 'exists'; id: string; path: string }
  | { type: 'watchFile'; id: string; path: string }
  | { type: 'unwatchFile'; id: string; path: string }
  | { type: 'watchDirectory'; id: string; path: string }
  | { type: 'unwatchDirectory'; id: string; path: string }
  | { type: 'toBytes'; id: string }
  | { type: 'forkToBytes'; id: string }
  | { type: 'loadBundle'; id: string; bundleBytes: ArrayBuffer; serverUrl?: string }
  | {
      type: 'initializeFromUrl';
      id: string;
      manifestUrl?: string;
      wasmUrl?: string;
      wsUrl?: string;
    }
  | {
      type: 'initializeFromBytes';
      id: string;
      bundleBytes: ArrayBuffer;
      serverUrl?: string;
      wsUrl?: string;
    }
  | { type: 'getServerUrl'; id: string }
  | { type: 'getManifest'; id: string }
  | { type: 'setAppSlug'; slug: string };

// Response types for VFS Worker communication (responses to client)
export type VFSWorkerResponse =
  // Operation responses (with id matching request)
  | { type: 'init'; success: boolean; error?: string }
  | {
      type: 'readFile';
      id: string;
      success: boolean;
      data?: DocumentData;
      error?: string;
    }
  | { type: 'writeFile'; id: string; success: boolean; error?: string }
  | { type: 'deleteFile'; id: string; success: boolean; error?: string }
  | { type: 'rename'; id: string; success: boolean; error?: string }
  | {
      type: 'listDirectory';
      id: string;
      success: boolean;
      data?: RefNode[];
      error?: string;
    }
  | {
      type: 'exists';
      id: string;
      success: boolean;
      data?: boolean;
      error?: string;
    }
  | { type: 'watchFile'; id: string; success: boolean; error?: string }
  | { type: 'unwatchFile'; id: string; success: boolean; error?: string }
  | { type: 'watchDirectory'; id: string; success: boolean; error?: string }
  | { type: 'unwatchDirectory'; id: string; success: boolean; error?: string }
  | {
      type: 'toBytes';
      id: string;
      success: boolean;
      data?: Uint8Array;
      rootId?: string;
      error?: string;
    }
  | {
      type: 'forkToBytes';
      id: string;
      success: boolean;
      data?: Uint8Array;
      rootId?: string;
      error?: string;
    }
  | { type: 'loadBundle'; id: string; success: boolean; error?: string }
  | { type: 'initializeFromUrl'; id: string; success: boolean; error?: string }
  | { type: 'initializeFromBytes'; id: string; success: boolean; error?: string }
  | {
      type: 'getServerUrl';
      id: string;
      success: boolean;
      data?: string;
      error?: string;
    }
  | {
      type: 'getManifest';
      id: string;
      success: boolean;
      data?: Manifest;
      error?: string;
    }
  // Watch event notifications (no id, matched by watchId)
  | { type: 'fileChanged'; watchId: string; documentData: DocumentData }
  | { type: 'directoryChanged'; watchId: string; path: string; changeData: unknown }
  // Connection status events (broadcast, no id)
  | { type: 'ready'; needsBundle?: boolean }
  | { type: 'disconnected' }
  | { type: 'reconnecting'; attempt: number }
  | { type: 'reconnected' }
  | { type: 'reconnectionFailed' }
  | { type: 'watchersReestablished'; count: number }
  // Service worker lifecycle events
  | { type: 'swReady'; autoInitialized: boolean; needsBundle?: boolean }
  | { type: 'swInitializing' }
  | { type: 'needsReinit'; appSlug: string | null; reason: string };

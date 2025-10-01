import { JsonValue, DocumentData, RefNode } from '@tonk/core';

export type DocumentContent = {
  content: JsonValue;
  bytes?: string;
};

// Message types for VFS Worker communication
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
  | { type: 'loadBundle'; id: string; bundleBytes: ArrayBuffer }
  | {
      type: 'initializeFromUrl';
      id: string;
      manifestUrl?: string;
      wasmUrl?: string;
      wsUrl?: string;
    };
export type VFSWorkerResponse =
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
  | { type: 'fileChanged'; watchId: string; documentData: DocumentData }
  | { type: 'watchDirectory'; id: string; success: boolean; error?: string }
  | { type: 'unwatchDirectory'; id: string; success: boolean; error?: string }
  | {
      type: 'directoryChanged';
      watchId: string;
      path: string;
      changeData: any;
    }
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
  | {
      type: 'loadBundle';
      id: string;
      success: boolean;
      error?: string;
    }
  | {
      type: 'initializeFromUrl';
      id: string;
      success: boolean;
      error?: string;
    };

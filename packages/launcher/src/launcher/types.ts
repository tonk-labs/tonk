import type {
  DocumentData,
  JsonValue,
  Manifest,
  RefNode,
} from "@tonk/core/slim";

/**
 * Represents the metadata of a Tonk bundle.
 * Used for listing bundles in the UI.
 */
export interface Bundle {
  id: string;
  name: string;
  size: number;
  createdAt: number;
  icon?: string;
}

/**
 * Represents the full data of a Tonk bundle, including its binary content.
 * Used for storage and runtime loading.
 */
export interface BundleData extends Bundle {
  bytes: Uint8Array;
  /** Cached full manifest to skip redundant Bundle.fromBytes in service worker */
  manifest?: Manifest;
}

/**
 * Configuration for the runtime environment.
 */
export interface RuntimeConfig {
  bundleId: string;
  operatorDid?: string;
}

/**
 * Messages sent from the UI/Client to the Service Worker.
 */
export type SWMessage =
  | { type: "ping" }
  | { type: "getBundle"; bundleId: string }
  | { type: "getBundleList" };

/**
 * Responses sent from the Service Worker to the UI/Client.
 */
export type SWResponse =
  | { type: "pong" }
  | { type: "bundle"; data: BundleData }
  | { type: "bundleList"; data: Bundle[] }
  | { type: "error"; message: string };

// --- VFS Types (Ported from packages/app/src/lib/types.ts) ---

export type DocumentContent = {
  content: JsonValue;
  bytes?: string;
};

// Message types for VFS Worker communication
export type VFSWorkerMessage =
  | { type: "init"; manifest: ArrayBuffer; wsUrl: string }
  | { type: "readFile"; id: string; path: string }
  | {
      type: "writeFile";
      id: string;
      path: string;
      content: DocumentContent;
      create: boolean;
    }
  | { type: "deleteFile"; id: string; path: string }
  | { type: "rename"; id: string; oldPath: string; newPath: string }
  | { type: "listDirectory"; id: string; path: string }
  | { type: "exists"; id: string; path: string }
  | { type: "watchFile"; id: string; path: string }
  | { type: "unwatchFile"; id: string; path: string }
  | { type: "watchDirectory"; id: string; path: string }
  | { type: "unwatchDirectory"; id: string; path: string }
  | { type: "toBytes"; id: string }
  | { type: "forkToBytes"; id: string }
  | {
      type: "loadBundle";
      id: string;
      bundleBytes: ArrayBuffer;
      serverUrl?: string;
      /** Cached full manifest to skip redundant Bundle.fromBytes in service worker */
      manifest?: Manifest;
      /** Unique IndexedDB bundle ID from launcher - used to differentiate bundles with same rootId */
      launcherBundleId?: string;
    }
  | {
      type: "initializeFromUrl";
      id: string;
      manifestUrl?: string;
      wasmUrl?: string;
      wsUrl?: string;
    }
  | {
      type: "initializeFromBytes";
      id: string;
      bundleBytes: ArrayBuffer;
      serverUrl?: string;
      wsUrl?: string;
    }
  | { type: "getServerUrl"; id: string }
  | { type: "setAppSlug"; slug: string }
  | { type: "handshake"; id: string }
  | { type: "getManifest"; id: string };

export type VFSWorkerResponse =
  | { type: "ready"; autoInitialized?: boolean; needsBundle?: boolean }
  | { type: "init"; success: boolean; error?: string }
  | { type: "handshake"; id: string; success: boolean; error?: string }
  | {
      type: "readFile";
      id: string;
      success: boolean;
      data?: DocumentData;
      error?: string;
    }
  | { type: "writeFile"; id: string; success: boolean; error?: string }
  | { type: "deleteFile"; id: string; success: boolean; error?: string }
  | { type: "rename"; id: string; success: boolean; error?: string }
  | {
      type: "listDirectory";
      id: string;
      success: boolean;
      data?: RefNode[];
      error?: string;
    }
  | {
      type: "exists";
      id: string;
      success: boolean;
      data?: boolean;
      error?: string;
    }
  | { type: "watchFile"; id: string; success: boolean; error?: string }
  | { type: "unwatchFile"; id: string; success: boolean; error?: string }
  | { type: "fileChanged"; watchId: string; documentData: DocumentData }
  | { type: "watchDirectory"; id: string; success: boolean; error?: string }
  | { type: "unwatchDirectory"; id: string; success: boolean; error?: string }
  | {
      type: "directoryChanged";
      watchId: string;
      path: string;
      // biome-ignore lint/suspicious/noExplicitAny: Change data structure is dynamic
      changeData: any;
    }
  | {
      type: "toBytes";
      id: string;
      success: boolean;
      data?: Uint8Array;
      rootId?: string;
      error?: string;
    }
  | {
      type: "forkToBytes";
      id: string;
      success: boolean;
      data?: Uint8Array;
      rootId?: string;
      error?: string;
    }
  | {
      type: "loadBundle";
      id: string;
      success: boolean;
      error?: string;
    }
  | {
      type: "initializeFromUrl";
      id: string;
      success: boolean;
      error?: string;
    }
  | {
      type: "initializeFromBytes";
      id: string;
      success: boolean;
      error?: string;
    }
  | { type: "setAppSlug"; success: boolean; error?: string }
  | {
      type: "getServerUrl";
      id: string;
      success: boolean;
      data?: string;
      error?: string;
    }
  | {
      type: "getManifest";
      id: string;
      success: boolean;
      // biome-ignore lint/suspicious/noExplicitAny: Manifest structure is dynamic
      data?: any;
      error?: string;
    }
  | { type: "disconnected" }
  | { type: "reconnecting"; attempt: number }
  | { type: "reconnected" }
  | { type: "reconnectionFailed" }
  | { type: "watchersReestablished"; count: number }
  | { type: "needsReinit"; appSlug: string | null; reason: string };

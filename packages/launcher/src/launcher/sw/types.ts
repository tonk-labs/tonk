import type { Manifest, TonkCore } from "@tonk/core/slim";

// Service Worker global scope types
declare global {
  interface ServiceWorkerGlobalScope {
    skipWaiting(): void;
    clients: Clients;
  }

  interface Clients {
    matchAll(options?: ClientQueryOptions): Promise<Client[]>;
    claim(): Promise<void>;
  }

  interface Client {
    postMessage(message: unknown): void;
    id: string;
  }

  interface ClientQueryOptions {
    includeUncontrolled?: boolean;
    type?: "window" | "worker" | "sharedworker" | "all";
  }

  interface ExtendableEvent extends Event {
    waitUntil(promise: Promise<unknown>): void;
  }
}

// Service Worker FetchEvent interface
export interface FetchEvent extends Event {
  request: Request;
  respondWith(response: Promise<Response> | Response): void;
}

// Bundle state for a single bundle instance
export type BundleState =
  | {
      status: "loading";
      launcherBundleId: string;
      bundleId: string;
      promise: Promise<void>;
    }
  | {
      status: "active";
      /** Root document ID from manifest */
      bundleId: string;
      /** Unique IndexedDB bundle ID from launcher - used to differentiate bundles with same rootId */
      launcherBundleId: string;
      tonk: TonkCore;
      manifest: Manifest;
      appSlug: string;
      wsUrl: string;
      healthCheckInterval: number | null;
      watchers: Map<string, { stop: () => void }>;
      connectionHealthy: boolean;
      reconnectAttempts: number;
    }
  | { status: "error"; launcherBundleId: string; error: Error };

// Helper type to extract active state
export type ActiveBundleState = Extract<BundleState, { status: "active" }>;

// Map of all loaded bundles, keyed by launcherBundleId
export type BundleStateMap = Map<string, BundleState>;

// Constants
export const MAX_RECONNECT_ATTEMPTS = 10;
export const HEALTH_CHECK_INTERVAL = 5000;

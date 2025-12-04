import type { TonkCore, Manifest } from '@tonk/core/slim';

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
  }

  interface ClientQueryOptions {
    includeUncontrolled?: boolean;
    type?: 'window' | 'worker' | 'sharedworker' | 'all';
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

// Bundle state machine types
export type BundleState =
  | { status: 'idle' }
  | { status: 'loading'; bundleId: string; promise: Promise<void> }
  | {
      status: 'active';
      bundleId: string;
      tonk: TonkCore;
      manifest: Manifest;
      appSlug: string;
      wsUrl: string;
      healthCheckInterval: number | null;
      watchers: Map<string, { stop: () => void }>;
      connectionHealthy: boolean;
      reconnectAttempts: number;
    }
  | { status: 'error'; error: Error; previousBundleId?: string };

// Helper type to extract active state
export type ActiveBundleState = Extract<BundleState, { status: 'active' }>;

// Constants
export const MAX_RECONNECT_ATTEMPTS = 10;
export const HEALTH_CHECK_INTERVAL = 5000;

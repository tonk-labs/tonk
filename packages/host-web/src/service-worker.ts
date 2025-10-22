/* eslint-env serviceworker */
/* eslint-disable @typescript-eslint/no-non-null-assertion */

import {
  TonkCore,
  Manifest,
  Bundle,
  initializeTonk,
  DocumentData,
} from '@tonk/core/slim';
import type { VFSWorkerMessage } from './types';

declare const TONK_SERVER_URL: string;
declare const TONK_SERVE_LOCAL: boolean;
declare const __SW_BUILD_TIME__: string;
declare const __SW_VERSION__: string;

interface FetchEvent extends Event {
  request: Request;
  respondWith(response: Promise<Response> | Response): void;
}

// Debug logging flag - set to true to enable comprehensive logging
const DEBUG_LOGGING = true;
console.log('🚀 SERVICE WORKER: Script loaded at', new Date().toISOString());
console.log('🔍 DEBUG_LOGGING enabled:', DEBUG_LOGGING);
console.log('🌐 Service worker location:', self.location.href);
console.log('🔧 SERVICE WORKER VERSION:', __SW_VERSION__);
console.log('🕐 SERVICE WORKER BUILD TIME:', __SW_BUILD_TIME__);

type TonkState =
  | { status: 'uninitialized' }
  | {
      status: 'loading';
      promise: Promise<{ tonk: TonkCore; manifest: Manifest }>;
    }
  | { status: 'ready'; tonk: TonkCore; manifest: Manifest }
  | { status: 'failed'; error: Error };

let tonkState: TonkState = { status: 'uninitialized' };
let appSlug: string | null = null; // Store the app slug for path resolution

let wsUrl: string | null = null;
let healthCheckInterval: number | null = null;
let connectionHealthy = true;
let reconnectAttempts = 0;
const MAX_RECONNECT_ATTEMPTS = 10;
const HEALTH_CHECK_INTERVAL = 5000;
const continuousRetryEnabled = true;

// Helper to get tonk instance when ready
function getTonk(): { tonk: TonkCore; manifest: Manifest } | null {
  return tonkState.status === 'ready' ? tonkState : null;
}

// Cache API helpers for persisting state across service worker restarts
const CACHE_NAME = 'tonk-sw-state-v1';
const APP_SLUG_URL = '/tonk-state/appSlug';
const BUNDLE_BYTES_URL = '/tonk-state/bundleBytes';

async function persistAppSlug(slug: string | null): Promise<void> {
  try {
    const cache = await caches.open(CACHE_NAME);

    if (slug === null) {
      await cache.delete(APP_SLUG_URL);
    } else {
      const response = new Response(JSON.stringify({ slug }), {
        headers: { 'Content-Type': 'application/json' },
      });
      await cache.put(APP_SLUG_URL, response);
    }

    log('info', 'AppSlug persisted to cache', { slug });
  } catch (error) {
    log('error', 'Failed to persist appSlug', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

async function restoreAppSlug(): Promise<string | null> {
  try {
    const cache = await caches.open(CACHE_NAME);
    const response = await cache.match(APP_SLUG_URL);

    if (!response) {
      return null;
    }

    const data = await response.json();
    return data.slug || null;
  } catch (error) {
    log('error', 'Failed to restore appSlug', {
      error: error instanceof Error ? error.message : String(error),
    });
    return null;
  }
}

async function persistBundleBytes(bytes: Uint8Array | null): Promise<void> {
  try {
    const cache = await caches.open(CACHE_NAME);

    if (bytes === null) {
      await cache.delete(BUNDLE_BYTES_URL);
    } else {
      // Convert Uint8Array to Blob for Response
      // Use type assertion to work around ArrayBufferLike type issue
      const blob = new Blob([bytes as any], {
        type: 'application/octet-stream',
      });
      const response = new Response(blob, {
        headers: { 'Content-Type': 'application/octet-stream' },
      });
      await cache.put(BUNDLE_BYTES_URL, response);
    }

    log('info', 'Bundle bytes persisted to cache', {
      size: bytes ? bytes.length : 0,
    });
  } catch (error) {
    log('error', 'Failed to persist bundle bytes', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

async function restoreBundleBytes(): Promise<Uint8Array | null> {
  try {
    const cache = await caches.open(CACHE_NAME);
    const response = await cache.match(BUNDLE_BYTES_URL);

    if (!response) {
      return null;
    }

    const arrayBuffer = await response.arrayBuffer();
    return new Uint8Array(arrayBuffer);
  } catch (error) {
    log('error', 'Failed to restore bundle bytes', {
      error: error instanceof Error ? error.message : String(error),
    });
    return null;
  }
}

// Logger utility
function log(level: any, message: any, data?: any) {
  if (!DEBUG_LOGGING) return;

  const timestamp = new Date().toISOString();
  const prefix = `[VFS Service Worker ${timestamp}] ${level.toUpperCase()}:`;

  if (data !== undefined) {
    // @ts-expect-error - dynamic console method access
    console[level](prefix, message, data);
  } else {
    // @ts-expect-error - dynamic console method access
    console[level](prefix, message);
  }
}

// Worker state for file watchers
const watchers = new Map();

// Helper to post messages back to main thread

async function postResponse(response: any) {
  log('info', 'Posting response to main thread', {
    type: response.type,
    success: 'success' in response ? response.success : 'N/A',
  });

  // Get all clients and post message to each
  const clients = await (self as any).clients.matchAll();

  clients.forEach((client: any) => {
    client.postMessage(response);
  });
}

async function performHealthCheck(): Promise<boolean> {
  const tonkInstance = getTonk();
  if (!tonkInstance) {
    return false;
  }

  try {
    const result = await tonkInstance.tonk.isConnected();
    return result;
  } catch (error) {
    console.error('🏥 [SW] performHealthCheck() ERROR:', error);
    return false;
  }
}

async function attemptReconnect() {
  if (!wsUrl) {
    log('error', 'Cannot reconnect: wsUrl not stored');
    console.error('🔄 [SW] Cannot reconnect: wsUrl not stored');
    return;
  }

  if (reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) {
    if (continuousRetryEnabled) {
      reconnectAttempts = 0;
    } else {
      log('error', 'Max reconnection attempts reached', {
        attempts: reconnectAttempts,
      });
      console.error(
        '🔄 [SW] Max reconnection attempts reached:',
        reconnectAttempts
      );
      await postResponse({ type: 'reconnectionFailed' });
      return;
    }
  }

  const tonkInstance = getTonk();
  if (!tonkInstance) {
    log('error', 'Cannot reconnect: tonk not initialized');
    console.error('🔄 [SW] Cannot reconnect: tonk not initialized');
    return;
  }

  reconnectAttempts++;
  log('info', 'Attempting to reconnect', {
    attempt: reconnectAttempts,
    maxAttempts: MAX_RECONNECT_ATTEMPTS,
    wsUrl,
  });

  await postResponse({ type: 'reconnecting', attempt: reconnectAttempts });

  try {
    await tonkInstance.tonk.connectWebsocket(wsUrl);

    await new Promise(resolve => setTimeout(resolve, 1000));

    const isConnected = await tonkInstance.tonk.isConnected();

    if (isConnected) {
      connectionHealthy = true;
      reconnectAttempts = 0;
      log('info', 'Reconnection successful');
      await postResponse({ type: 'reconnected' });

      await reestablishWatchers();
    } else {
      throw new Error('Connection check failed after reconnect attempt');
    }
  } catch (error) {
    log('error', 'Reconnection failed', {
      error: error instanceof Error ? error.message : String(error),
      attempt: reconnectAttempts,
    });

    const backoffDelay = Math.min(
      1000 * Math.pow(2, reconnectAttempts - 1),
      30000
    );
    log('info', 'Scheduling next reconnect attempt', {
      delayMs: backoffDelay,
      nextAttempt: reconnectAttempts + 1,
    });
    setTimeout(attemptReconnect, backoffDelay);
  }
}

async function reestablishWatchers() {
  log('info', 'Re-establishing watchers after reconnection', {
    watcherCount: watchers.size,
  });

  const tonkInstance = getTonk();
  if (!tonkInstance) {
    log('error', 'Cannot re-establish watchers: tonk not available');
    return;
  }

  const watcherInfo = Array.from(watchers.entries());

  log('info', 'Watcher re-establishment complete', {
    watcherCount: watcherInfo.length,
  });

  await postResponse({
    type: 'watchersReestablished',
    count: watcherInfo.length,
  });
}

function startHealthMonitoring() {
  if (healthCheckInterval) {
    clearInterval(healthCheckInterval);
  }

  log('info', 'Starting health monitoring', {
    intervalMs: HEALTH_CHECK_INTERVAL,
  });

  healthCheckInterval = setInterval(async () => {
    const isHealthy = await performHealthCheck();

    if (!isHealthy && connectionHealthy) {
      connectionHealthy = false;
      log('error', 'Connection lost, starting reconnection attempts');
      console.error('❌ [SW] Connection lost, starting reconnection attempts');
      await postResponse({ type: 'disconnected' });
      attemptReconnect();
    } else if (isHealthy && !connectionHealthy) {
      connectionHealthy = true;
      reconnectAttempts = 0;
      log('info', 'Connection health restored');
    }
  }, HEALTH_CHECK_INTERVAL) as unknown as number;
}

// Auto-initialize Tonk from cache if available
log('info', 'Service worker starting, checking for cached state');
console.log('🚀 SERVICE WORKER: Starting initialization');
tonkState = { status: 'uninitialized' };

// Auto-initialization function
async function autoInitializeFromCache() {
  try {
    // Try to restore appSlug and bundle from cache
    const restoredSlug = await restoreAppSlug();
    const bundleBytes = await restoreBundleBytes();

    if (!restoredSlug || !bundleBytes) {
      log('info', 'No cached state found, waiting for initialization message', {
        hasSlug: !!restoredSlug,
        hasBundle: !!bundleBytes,
      });
      console.log('🔄 SERVICE WORKER: No cached state - waiting for bundle');
      return;
    }

    // Found cached state - auto-initialize
    appSlug = restoredSlug;
    log('info', 'Found cached state, auto-initializing', {
      slug: appSlug,
      bundleSize: bundleBytes.length,
    });
    console.log('🔄 SERVICE WORKER: Auto-initializing from cache...', appSlug);

    // Initialize WASM from server
    const wasmUrl = `${TONK_SERVER_URL}/tonk_core_bg.wasm`;
    log('info', 'Fetching WASM from server');
    await initializeTonk({ wasmPath: wasmUrl });
    log('info', 'WASM initialization completed');

    // Create bundle and manifest
    const bundle = await Bundle.fromBytes(bundleBytes);
    const manifest = await bundle.getManifest();
    log('info', 'Bundle and manifest restored from cache', {
      rootId: manifest.rootId,
    });

    // Create TonkCore instance
    const tonk = await TonkCore.fromBytes(bundleBytes, {
      storage: { type: 'indexeddb' },
    });
    log('info', 'TonkCore created from cached bundle');

    // Connect to websocket
    wsUrl = `${TONK_SERVER_URL.replace(/^http/, 'ws')}`;
    log('info', 'Connecting to websocket...', {
      wsUrl,
      localRootId: manifest.rootId,
    });
    await tonk.connectWebsocket(wsUrl);
    log('info', 'Websocket connected');
    startHealthMonitoring();

    // Wait for initial sync
    let syncAttempts = 0;
    const maxAttempts = 20;
    while (syncAttempts < maxAttempts) {
      try {
        await tonk.listDirectory('/');
        log('info', 'Auto-initialization complete - tonk ready');
        console.log('✅ SERVICE WORKER: Auto-initialized successfully');
        break;
        // eslint-disable-next-line @typescript-eslint/no-unused-vars
      } catch (_error) {
        syncAttempts++;
        if (syncAttempts >= maxAttempts) {
          log('warn', 'Initial sync timeout after restore', {
            attempts: syncAttempts,
          });
        } else {
          await new Promise(resolve => setTimeout(resolve, 500));
        }
      }
    }

    // Update tonk state
    tonkState = { status: 'ready', tonk, manifest };
  } catch (error) {
    log('error', 'Auto-initialization failed', {
      error: error instanceof Error ? error.message : String(error),
    });
    console.error('❌ SERVICE WORKER: Auto-initialization failed:', error);

    // Reset to uninitialized state
    tonkState = { status: 'uninitialized' };
    appSlug = null;

    // Notify clients that manual initialization is needed
    const allClients = await (self as any).clients.matchAll();

    allClients.forEach((client: any) => {
      client.postMessage({
        type: 'needsReinit',
        appSlug: null,
        reason: 'Auto-initialization failed',
      });
    });
  }
}

// Start auto-initialization (non-blocking)
autoInitializeFromCache();

self.addEventListener('install', _event => {
  log('info', 'Service worker installing');
  console.log('🔧 SERVICE WORKER: Installing SW');
  (self as any).skipWaiting();
  log('info', 'Service worker install completed, skipWaiting called');
});

// Consolidated activate handler with proper event.waitUntil for Safari compatibility
self.addEventListener('activate', event => {
  log('info', 'Service worker activating');
  console.log('🚀 SERVICE WORKER: Activating service worker.');

  // Use waitUntil to ensure Safari waits for async operations to complete
  (event as any).waitUntil(
    (async () => {
      // Claim clients first to take control of the page
      await (self as any).clients.claim();
      log('info', 'Service worker activation completed, clients claimed');

      // Then notify all clients that service worker is ready
      const allClients = await (self as any).clients.matchAll();

      allClients.forEach((client: any) => {
        client.postMessage({ type: 'ready', needsBundle: true });
      });
      log(
        'info',
        'Service worker activated, ready message sent to all clients'
      );
      console.log('🚀 SERVICE WORKER: Activated and ready');
    })()
  );
});

const targetToResponse = async (target: any): Promise<Response> => {
  if (target.bytes) {
    // target.bytes is a base64 string, decode it to binary for Response
    const binaryString = atob(target.bytes);
    const bytes = new Uint8Array(binaryString.length);
    for (let i = 0; i < binaryString.length; i++) {
      bytes[i] = binaryString.charCodeAt(i);
    }
    return new Response(bytes, {
      headers: { 'Content-Type': target.content.mime },
    });
  } else {
    return new Response(target.content, {
      headers: { 'Content-Type': 'application/json' },
    });
  }
};

const determinePath = (url: URL): string => {
  console.log('🎯 determinePath START', {
    url: url.href,
    pathname: url.pathname,
    appSlug: appSlug || 'none',
  });

  // If no appSlug is set, we can't determine the path
  if (!appSlug) {
    console.error('determinePath - NO APP SLUG SET');
    throw new Error(`No app slug available for ${url.pathname}`);
  }

  // Strip the scope from the pathname
  const scopePath = new URL(
    // @ts-expect-error - self.registration exists in ServiceWorkerGlobalScope
    (self.registration?.scope ?? self.location.href) as string
  ).pathname;
  const strippedPath = url.pathname.startsWith(scopePath)
    ? url.pathname.slice(scopePath.length)
    : url.pathname;

  // Remove leading slashes and split into segments
  const segments = strippedPath.replace(/^\/+/, '').split('/').filter(Boolean);

  console.log('determinePath - segments', {
    scopePath,
    strippedPath,
    segments: [...segments],
    firstSegment: segments[0] || 'none',
  });

  // Check if the appSlug is already at the start
  let pathSegments = segments;
  if (segments[0] === appSlug) {
    // AppSlug is already present, remove it from segments
    pathSegments = segments.slice(1);
    console.log(
      'determinePath - appSlug already present, using remaining segments',
      {
        pathSegments: [...pathSegments],
      }
    );
  } else {
    console.log(
      'determinePath - appSlug not present, using all segments as path',
      {
        pathSegments: [...pathSegments],
      }
    );
  }

  // If no segments left or path ends with slash, default to index.html
  if (pathSegments.length === 0 || url.pathname.endsWith('/')) {
    const result = `${appSlug}/index.html`;
    console.log('determinePath - defaulting to index.html', { result });
    return result;
  }

  // Regular file path
  const result = `${appSlug}/${pathSegments.join('/')}`;
  console.log('determinePath - returning file path', { result });
  return result;
};

(self as any).addEventListener('fetch', (event: FetchEvent) => {
  const url = new URL(event.request.url);
  const referrer = event.request.referrer;

  console.log(
    '🔥 FETCH EVENT:',
    url.href,
    'Origin match:',
    url.origin === location.origin,
    'Pathname:',
    url.pathname
  );
  console.log('🔥 Tonk state:', tonkState.status);
  console.log('🔥 Current appSlug:', appSlug);

  log('info', 'Fetch event received', {
    url: url.href,
    origin: url.origin,
    pathname: url.pathname,
    referrer: referrer,
    method: event.request.method,
    matchesOrigin: url.origin === location.origin,
  });

  // Check for WebSocket upgrade requests for Vite HMR
  const upgradeHeader = event.request.headers.get('upgrade');
  if (upgradeHeader && upgradeHeader.toLowerCase() === 'websocket') {
    log(
      'info',
      '🔌 WS: WebSocket upgrade request detected - allowing through',
      {
        url: url.href,
      }
    );
    // Don't intercept WebSocket connections - let them pass through
    return;
  }

  // Check if this is a request for the root hostname (should bypass VFS)
  const isRootRequest = url.pathname === '/' || url.pathname === '';
  if (isRootRequest && appSlug) {
    appSlug = null;
    // Persist the reset so it survives service worker restarts
    // Also clear bundle bytes to avoid stale data
    Promise.all([persistAppSlug(null), persistBundleBytes(null)]).catch(err => {
      log('error', 'Failed to persist state reset', { error: err });
    });
  }

  if (TONK_SERVE_LOCAL && appSlug && !isRootRequest) {
    // Intercept and proxy Vite HMR assets
    if (
      url.pathname.startsWith('/@vite') ||
      url.pathname.startsWith('/@react-refresh') ||
      url.pathname.startsWith('/src/') ||
      url.pathname.startsWith(`/${appSlug}/@vite`) ||
      url.pathname.startsWith(`/${appSlug}/node_modules`) ||
      url.pathname.startsWith(`/${appSlug}/src/`) ||
      url.pathname.startsWith('/node_modules') ||
      url.pathname.includes('__vite__') ||
      url.searchParams.has('t') // cache-busting params
    ) {
      log('info', 'Vite HMR asset detected, proxying to dev server', {
        url: url.href,
      });

      event.respondWith(
        (async () => {
          try {
            // Keep the full path including appSlug for Vite to use its base config :
            const localDevUrl = `http://localhost:4001${url.pathname}${url.search}`;

            log('info', 'Fetching Vite asset from dev server', {
              original: url.pathname,
              proxied: localDevUrl,
            });

            const response = await fetch(localDevUrl);

            // Add no-cache headers to prevent browser caching during development
            const headers = new Headers(response.headers);
            headers.set('Cache-Control', 'no-cache, no-store, must-revalidate');
            headers.set('Pragma', 'no-cache');
            headers.set('Expires', '0');

            return new Response(response.body, {
              status: response.status,
              statusText: response.statusText,
              headers: headers,
            });
          } catch (err) {
            log('error', 'Failed to fetch Vite asset', {
              error: err instanceof Error ? err.message : String(err),
              url: url.pathname,
            });
            return new Response('Vite dev server unreachable', {
              status: 502,
              headers: { 'Content-Type': 'text/plain' },
            });
          }
        })()
      );
      return;
    }

    log('info', 'TONK_SERVE_LOCAL enabled, proxying to local dev server');

    event.respondWith(
      (async () => {
        try {
          // Keep the full path including appSlug for Vite to use its base config
          const localDevUrl = `http://localhost:4001${url.pathname}${url.search}`;

          log('info', 'Proxying request to local dev server', {
            original: url.pathname,
            proxied: localDevUrl,
          });

          const response = await fetch(localDevUrl);
          log('info', 'Received response from local dev server', {
            status: response.status,
            contentType: response.headers.get('content-type'),
          });

          // Add no-cache headers to prevent browser caching during development
          const headers = new Headers(response.headers);
          headers.set('Cache-Control', 'no-cache, no-store, must-revalidate');
          headers.set('Pragma', 'no-cache');
          headers.set('Expires', '0');

          return new Response(response.body, {
            status: response.status,
            statusText: response.statusText,
            headers: headers,
          });
        } catch (err) {
          log('error', 'Failed to fetch from local dev server', {
            error: err instanceof Error ? err.message : String(err),
            url: url.pathname,
          });
          return new Response('Local dev server unreachable on port 4001', {
            status: 502,
            headers: { 'Content-Type': 'text/plain' },
          });
        }
      })()
    );
    return; // Exit early, don't process VFS
  }

  if (appSlug && url.origin === location.origin && !isRootRequest) {
    log('info', 'Processing fetch request for same origin (non-root)');
    event.respondWith(
      (async () => {
        try {
          const path = determinePath(url);
          log('info', 'Determined path for request', {
            path,
            originalUrl: url.href,
          });

          const tonkInstance = getTonk();
          log('info', 'Retrieved Tonk instance', {
            hasTonkInstance: !!tonkInstance,
            tonkState: tonkState.status,
            tonkStateDetails:
              tonkState.status === 'ready'
                ? 'ready'
                : tonkState.status === 'loading'
                  ? 'loading'
                  : tonkState.status === 'failed'
                    ? 'failed'
                    : 'uninitialized',
          });

          if (!tonkInstance) {
            log('error', 'Tonk not initialized - cannot handle request', {
              tonkState: tonkState.status,
              path,
              url: url.href,
            });
            throw new Error('Tonk not initialized');
          }

          // Check if the file exists first
          const filePath = `/${path}`;
          const exists = await tonkInstance.tonk.exists(filePath);

          if (!exists) {
            console.warn(
              `🚨 File not found: ${filePath}, falling back to index.html`
            );
            log('warn', 'File not found, falling back to index.html', {
              requestedPath: filePath,
              fallbackPath: `/${appSlug}/index.html`,
            });
            // Fall back to index.html
            const indexPath = `/${appSlug}/index.html`;
            const target = await tonkInstance.tonk.readFile(indexPath);
            log('info', 'Successfully read index.html fallback', {
              filePath: indexPath,
              hasContent: !!target.content,
              hasBytes: !!target.bytes,
            });
            return targetToResponse(target);
          }

          log('info', 'File exists, attempting to read from Tonk', {
            filePath,
          });
          const target = await tonkInstance.tonk.readFile(filePath);
          log('info', 'Successfully read file from Tonk', {
            filePath,
            hasContent: !!target.content,
            hasBytes: !!target.bytes,
            contentType: target.content
              ? (target.content! as any).mime
              : 'unknown',
          });

          const response = targetToResponse(target);
          log('info', 'Created response for file', {
            filePath: `/${path}`,
            responseType: target.bytes ? 'binary' : 'text',
          });
          return response;
        } catch (e) {
          log(
            'error',
            'Failed to fetch file from Tonk, falling back to original request',
            {
              error: e instanceof Error ? e.message : String(e),
              path: determinePath(url),
              url: url.href,
              tonkState: tonkState.status,
            }
          );

          // Instead of returning a 404, fall back to the original request
          // This prevents redirect loops when files don't exist in VFS
          log('info', 'Falling back to original fetch request', {
            url: url.href,
          });
          return fetch(event.request);
        }
      })()
    );
  } else {
    log('info', 'Ignoring fetch request for different origin', {
      requestOrigin: url.origin,
      serviceWorkerOrigin: location.origin,
    });
  }
});

async function handleMessage(
  message: VFSWorkerMessage | { type: 'setAppSlug'; slug: string }
) {
  log('info', 'Received message', {
    type: message.type,
    id: 'id' in message ? message.id : 'N/A',
  });

  // Handle setAppSlug message separately
  if (message.type === 'setAppSlug') {
    appSlug = message.slug;

    // Persist to IndexedDB so it survives service worker restarts
    await persistAppSlug(appSlug);

    log('info', 'App slug set and persisted', { slug: appSlug });
    return;
  }

  // Allow init, loadBundle, and initializeFromUrl operations when not ready
  const allowedWhenUninitialized = [
    'init',
    'loadBundle',
    'initializeFromUrl',
    'getServerUrl',
  ];
  if (
    tonkState.status !== 'ready' &&
    !allowedWhenUninitialized.includes(message.type)
  ) {
    log('error', 'Operation attempted before VFS initialization', {
      type: message.type,
      status: tonkState.status,
    });
    if ('id' in message) {
      postResponse({
        type: message.type,
        id: message.id,
        success: false,
        error: 'VFS not initialized. Please load a bundle first.',
      });
    }
    return;
  }

  switch (message.type) {
    case 'init':
      log('info', 'Handling VFS init message', {
        manifestSize: message.manifest.byteLength,
        wsUrl: message.wsUrl,
        id: 'id' in message ? message.id : 'N/A',
      });
      try {
        // Check if we already have a Tonk instance
        if (tonkState.status === 'ready') {
          log('info', 'Tonk already initialized, responding with success');
          postResponse({
            type: 'init',
            success: true,
          });
          return;
        }

        // If Tonk is still loading, wait for it
        if (tonkState.status === 'loading') {
          log('info', 'Tonk is loading, waiting for completion');
          try {
            await tonkState.promise;
            log('info', 'Tonk loading completed, responding with success');
            postResponse({
              type: 'init',
              success: true,
            });
          } catch (error) {
            log('error', 'Tonk loading failed', {
              error: error instanceof Error ? error.message : String(error),
            });
            postResponse({
              type: 'init',
              success: false,
              error: error instanceof Error ? error.message : String(error),
            });
          }
          return;
        }

        // If Tonk failed to initialize, respond with error
        if (tonkState.status === 'failed') {
          log('error', 'Tonk initialization failed previously', {
            error: tonkState.error.message,
          });
          postResponse({
            type: 'init',
            success: false,
            error: tonkState.error.message,
          });
          return;
        }

        // If uninitialized, this shouldn't happen in normal flow
        log('warn', 'Tonk is uninitialized, this is unexpected');
        postResponse({
          type: 'init',
          success: false,
          error: 'Tonk not initialized',
        });
      } catch (error) {
        log('error', 'Failed to handle init message', {
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'init',
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'readFile':
      log('info', 'Reading file', { path: message.path, id: message.id });
      try {
        const { tonk } = getTonk()!;
        const documentData = await tonk!.readFile(message.path);
        log('info', 'File read successfully', {
          path: message.path,
          documentType: documentData.type,
          hasBytes: !!documentData.bytes,
          contentType: typeof documentData.content,
        });

        postResponse({
          type: 'readFile',
          id: message.id,
          success: true,
          data: documentData,
        });
      } catch (error) {
        log('error', 'Failed to read file', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'readFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'writeFile':
      log('info', 'Writing file', {
        path: message.path,
        id: message.id,
        create: message.create,
        hasBytes: !!message.content.bytes,
        contentType: typeof message.content.content,
      });
      try {
        const { tonk } = getTonk()!;
        if (message.create) {
          log('info', 'Creating new file', { path: message.path });
          if (message.content.bytes) {
            // Create file with bytes
            await tonk.createFileWithBytes(
              message.path,
              message.content.content,
              message.content.bytes
            );
          } else {
            // Create file with content only
            await tonk.createFile(message.path, message.content.content);
          }
        } else {
          log('info', 'Updating existing file', { path: message.path });
          if (message.content.bytes) {
            // Update file with bytes
            await tonk.updateFileWithBytes(
              message.path,
              message.content.content,
              message.content.bytes
            );
          } else {
            // Update file with content only
            await tonk.updateFile(message.path, message.content.content);
          }
        }
        log('info', 'File write completed successfully', {
          path: message.path,
        });
        postResponse({
          type: 'writeFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to write file', {
          path: message.path,
          create: message.create,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'writeFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'deleteFile':
      log('info', 'Deleting file', { path: message.path, id: message.id });
      try {
        const { tonk } = getTonk()!;
        await tonk.deleteFile(message.path);
        log('info', 'File deleted successfully', { path: message.path });
        postResponse({
          type: 'deleteFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to delete file', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'deleteFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'rename':
      log('info', 'Renaming file or directory', {
        oldPath: message.oldPath,
        newPath: message.newPath,
        id: message.id,
      });
      try {
        const { tonk } = getTonk()!;
        await tonk.rename(message.oldPath, message.newPath);
        log('info', 'Rename completed successfully', {
          oldPath: message.oldPath,
          newPath: message.newPath,
        });
        postResponse({
          type: 'rename',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to rename', {
          oldPath: message.oldPath,
          newPath: message.newPath,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'rename',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'listDirectory':
      log('info', 'Listing directory', { path: message.path, id: message.id });
      try {
        const { tonk } = getTonk()!;
        const files = await tonk.listDirectory(message.path);
        log('info', 'Directory listed successfully', {
          path: message.path,
          fileCount: Array.isArray(files) ? files.length : 'unknown',
        });
        // Pass through the raw response from TonkCore
        postResponse({
          type: 'listDirectory',
          id: message.id,
          success: true,
          data: files,
        });
      } catch (error) {
        log('error', 'Failed to list directory', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'listDirectory',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'exists':
      log('info', 'Checking file existence', {
        path: message.path,
        id: message.id,
      });
      try {
        const { tonk } = getTonk()!;
        const exists = await tonk.exists(message.path);
        log('info', 'File existence check completed', {
          path: message.path,
          exists,
        });
        postResponse({
          type: 'exists',
          id: message.id,
          success: true,
          data: exists,
        });
      } catch (error) {
        log('error', 'Failed to check file existence', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'exists',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'watchFile':
      log('info', 'Starting file watch', {
        path: message.path,
        id: message.id,
      });
      try {
        const { tonk } = getTonk()!;
        const watcher = await tonk!.watchFile(
          message.path,
          (documentData: DocumentData) => {
            log('info', 'File change detected', {
              watchId: message.id,
              path: message.path,
              documentType: documentData.type,
              hasBytes: !!documentData.bytes,
            });

            postResponse({
              type: 'fileChanged',
              watchId: message.id,
              documentData: documentData,
            });
          }
        );
        watchers.set(message.id, watcher!);
        log('info', 'File watch started successfully', {
          path: message.path,
          watchId: message.id,
          totalWatchers: watchers.size,
        });
        postResponse({
          type: 'watchFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to start file watch', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'watchFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'unwatchFile':
      log('info', 'Stopping file watch', { watchId: message.id });
      try {
        const watcher = watchers.get(message.id);
        if (watcher) {
          log('info', 'Found watcher, stopping it', { watchId: message.id });
          watcher.stop();
          watchers.delete(message.id);
          log('info', 'File watch stopped successfully', {
            watchId: message.id,
            remainingWatchers: watchers.size,
          });
        } else {
          log('warn', 'No watcher found for ID', { watchId: message.id });
        }
        postResponse({
          type: 'unwatchFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to stop file watch', {
          watchId: message.id,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'unwatchFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'watchDirectory':
      log('info', 'Starting directory watch', {
        path: message.path,
        id: message.id,
      });
      try {
        const { tonk } = getTonk()!;
        const watcher = await tonk!.watchDirectory(
          message.path,
          (changeData: any) => {
            log('info', 'Directory change detected', {
              watchId: message.id,
              path: message.path,
            });

            postResponse({
              type: 'directoryChanged',
              watchId: message.id,
              path: message.path,
              changeData,
            });
          }
        );
        watchers.set(message.id, watcher!);
        log('info', 'Directory watch started successfully', {
          path: message.path,
          watchId: message.id,
          totalWatchers: watchers.size,
        });
        postResponse({
          type: 'watchDirectory',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to start directory watch', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'watchDirectory',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'unwatchDirectory':
      log('info', 'Stopping directory watch', { watchId: message.id });
      try {
        const watcher = watchers.get(message.id);
        if (watcher) {
          log('info', 'Found directory watcher, stopping it', {
            watchId: message.id,
          });
          watcher.stop();
          watchers.delete(message.id);
          log('info', 'Directory watch stopped successfully', {
            watchId: message.id,
            remainingWatchers: watchers.size,
          });
        } else {
          log('warn', 'No directory watcher found for ID', {
            watchId: message.id,
          });
        }
        postResponse({
          type: 'unwatchDirectory',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to stop directory watch', {
          watchId: message.id,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'unwatchDirectory',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'toBytes':
      log('info', 'Converting tonk to bytes', { id: message.id });
      try {
        const { tonk, manifest } = getTonk()!;
        const bytes = await tonk.toBytes();
        const rootId = manifest.rootId;
        log('info', 'Tonk converted to bytes successfully', {
          id: message.id,
          byteLength: bytes.length,
          rootId,
          manifestKeys: Object.keys(manifest),
        });
        postResponse({
          type: 'toBytes',
          id: message.id,
          success: true,
          data: bytes,
          rootId,
        });
      } catch (error) {
        log('error', 'Failed to convert tonk to bytes', {
          id: message.id,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'toBytes',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'forkToBytes':
      log('info', 'Forking tonk to bytes', { id: message.id });
      try {
        const { tonk } = getTonk()!;
        const bytes = await tonk.forkToBytes();

        // Create a new bundle from the forked bytes to get the new rootId
        const forkedBundle = await Bundle.fromBytes(bytes);
        const forkedManifest = await forkedBundle.getManifest();
        const rootId = forkedManifest.rootId;

        log('info', 'Tonk forked to bytes successfully', {
          id: message.id,
          byteLength: bytes.length,
          rootId,
          manifestKeys: Object.keys(forkedManifest),
        });
        postResponse({
          type: 'forkToBytes',
          id: message.id,
          success: true,
          data: bytes,
          rootId,
        });
      } catch (error) {
        log('error', 'Failed to fork tonk to bytes', {
          id: message.id,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'forkToBytes',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'loadBundle':
      log('info', 'Loading new bundle', {
        id: message.id,
        byteLength: message.bundleBytes.byteLength,
        serverUrl: (message as any).serverUrl,
      });
      try {
        // Get server URL from message or fall back to TONK_SERVER_URL
        const serverUrl = (message as any).serverUrl || TONK_SERVER_URL;
        log('info', 'Using server URL', { serverUrl });

        // Initialize WASM if not already initialized
        if (tonkState.status === 'uninitialized') {
          log('info', 'WASM not initialized, initializing now');
          const wasmUrl = `${serverUrl}/tonk_core_bg.wasm`;
          const cacheBustUrl = `${wasmUrl}?t=${Date.now()}`;
          log('info', 'Fetching WASM from', { cacheBustUrl });
          await initializeTonk({ wasmPath: cacheBustUrl });
          log('info', 'WASM initialization completed for bundle loading');
        }

        // Convert ArrayBuffer to Uint8Array
        const bundleBytes = new Uint8Array(message.bundleBytes);
        log('info', 'Creating bundle from bytes', {
          byteLength: bundleBytes.length,
        });

        // Create new bundle and manifest
        const bundle = await Bundle.fromBytes(bundleBytes);
        const manifest = await bundle.getManifest();
        log('info', 'Bundle and manifest created successfully', {
          rootId: manifest.rootId,
        });

        // Create new TonkCore instance
        log('info', 'Creating new TonkCore from bundle bytes');
        const newTonk = await TonkCore.fromBytes(bundleBytes, {
          storage: { type: 'indexeddb' },
        });
        log('info', 'New TonkCore created successfully');

        // Get the websocket URL from the current config (serverUrl already defined above)
        const urlParams = new URLSearchParams(self.location.search);
        const bundleParam = urlParams.get('bundle');
        let wsUrlLocal = `${serverUrl.replace(/^http/, 'ws')}`;

        if (bundleParam) {
          try {
            const decoded = atob(bundleParam);
            const bundleConfig = JSON.parse(decoded);
            if (bundleConfig.wsUrl) {
              wsUrlLocal = bundleConfig.wsUrl;
            }
          } catch (error) {
            log('warn', 'Could not parse bundle config for wsUrl', {
              error: error instanceof Error ? error.message : String(error),
            });
          }
        }

        log('info', 'Determined websocket URL', {
          wsUrl: wsUrlLocal,
          serverUrl,
        });

        // Connect to websocket
        wsUrl = wsUrlLocal;
        log('info', 'Connecting new tonk to websocket', {
          wsUrl,
          localRootId: manifest.rootId,
        });
        if (wsUrl) {
          await newTonk.connectWebsocket(wsUrl);
          log('info', 'Websocket connection established');
          startHealthMonitoring();
        }

        // Wait for initial data sync by polling until root is accessible
        log('info', 'Waiting for initial data sync');
        let syncAttempts = 0;
        const maxAttempts = 20; // 10 seconds max
        while (syncAttempts < maxAttempts) {
          try {
            // Try to list the root directory to verify data is synced
            await newTonk.listDirectory('/');
            log('info', 'Initial data sync confirmed');
            break;
          } catch (error) {
            syncAttempts++;
            if (syncAttempts >= maxAttempts) {
              log('warn', 'Initial sync timeout, proceeding anyway', {
                attempts: syncAttempts,
                error: error instanceof Error ? error.message : String(error),
              });
            } else {
              // Wait 500ms before trying again
              await new Promise(resolve => setTimeout(resolve, 500));
            }
          }
        }

        // Update the global tonk state
        tonkState = { status: 'ready', tonk: newTonk, manifest };
        log('info', 'Tonk state updated with new instance');

        // Persist bundle bytes to survive service worker restarts
        await persistBundleBytes(bundleBytes);
        log('info', 'Bundle bytes persisted to IndexedDB');

        postResponse({
          type: 'loadBundle',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to load bundle', {
          id: message.id,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'loadBundle',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'getServerUrl':
      log('info', 'Getting server URL', { id: message.id });
      postResponse({
        type: 'getServerUrl',
        id: message.id,
        success: true,
        data: TONK_SERVER_URL,
      });
      break;

    case 'initializeFromUrl':
      log('info', 'Initializing from URL', {
        id: message.id,
        manifestUrl: message.manifestUrl,
        wasmUrl: message.wasmUrl,
      });
      try {
        // Extract URLs from message, with defaults
        const wasmUrl =
          message.wasmUrl || `${TONK_SERVER_URL}/tonk_core_bg.wasm`;
        const manifestUrl =
          message.manifestUrl || `${TONK_SERVER_URL}/.manifest.tonk`;
        const wsUrlInit =
          message.wsUrl || TONK_SERVER_URL.replace(/^http/, 'ws');

        log('info', 'Fetching WASM from URL', { wasmUrl });

        // Initialize WASM with cache-busting
        const cacheBustUrl = `${wasmUrl}?t=${Date.now()}`;
        await initializeTonk({ wasmPath: cacheBustUrl });
        log('info', 'WASM initialization completed');

        // Fetch the manifest data
        log('info', 'Fetching manifest from URL', { manifestUrl });
        const manifestResponse = await fetch(manifestUrl);
        const manifestBytes = new Uint8Array(
          await manifestResponse.arrayBuffer()
        );
        log('info', 'Manifest bytes loaded', {
          byteLength: manifestBytes.length,
        });

        // Create bundle and manifest
        const bundle = await Bundle.fromBytes(manifestBytes);
        const manifest = await bundle.getManifest();
        log('info', 'Bundle and manifest created successfully');

        // Create TonkCore instance
        log('info', 'Creating TonkCore from manifest bytes');
        const tonk = await TonkCore.fromBytes(manifestBytes, {
          storage: { type: 'indexeddb' },
        });
        log('info', 'TonkCore created successfully');

        // Connect to websocket
        wsUrl = wsUrlInit;
        log('info', 'Connecting to websocket', { wsUrl });
        await tonk.connectWebsocket(wsUrl);
        log('info', 'Websocket connection established');
        startHealthMonitoring();

        // Wait for initial data sync by polling until root is accessible
        log('info', 'Waiting for initial data sync');
        let syncAttempts = 0;
        const maxAttempts = 20; // 10 seconds max
        while (syncAttempts < maxAttempts) {
          try {
            // Try to list the root directory to verify data is synced
            await tonk.listDirectory('/');
            log('info', 'Initial data sync confirmed');
            break;
          } catch (error) {
            syncAttempts++;
            if (syncAttempts >= maxAttempts) {
              log('warn', 'Initial sync timeout, proceeding anyway', {
                attempts: syncAttempts,
                error: error instanceof Error ? error.message : String(error),
              });
            } else {
              // Wait 500ms before trying again
              await new Promise(resolve => setTimeout(resolve, 500));
            }
          }
        }

        // Update the global tonk state
        tonkState = { status: 'ready', tonk, manifest };
        log('info', 'Tonk state updated to ready from URL');

        // Persist bundle bytes to survive service worker restarts
        await persistBundleBytes(manifestBytes);
        log('info', 'Bundle bytes persisted to IndexedDB');

        postResponse({
          type: 'initializeFromUrl',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to initialize from URL', {
          id: message.id,
          error: error instanceof Error ? error.message : String(error),
        });
        tonkState = { status: 'failed', error: error as Error };
        postResponse({
          type: 'initializeFromUrl',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;
  }
}

// Listen for messages from main thread (for VFS operations)
self.addEventListener('message', async event => {
  log('info', 'Raw message event received', {
    eventType: event.type,
    messageType: event.data?.type,
    hasData: !!event.data,
  });
  try {
    await handleMessage(event.data as VFSWorkerMessage);
  } catch (error) {
    log('error', 'Error handling message', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
});

// Worker startup
log('info', 'VFS Service Worker started', { debugLogging: DEBUG_LOGGING });

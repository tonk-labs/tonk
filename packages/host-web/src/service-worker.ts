/* eslint-env serviceworker */
/* global self, console, fetch, atob, btoa, caches, clients, location, URL, Response, __DEV_MODE__ */

console.log('🚀 SERVICE WORKER: Script loaded at', new Date().toISOString());
import {
  TonkCore,
  Manifest,
  Bundle,
  initializeTonk,
  DocumentData,
} from '@tonk/core/slim';
import mime from 'mime';
import type { VFSWorkerMessage } from './types';

declare const __DEV_MODE__: boolean;

interface FetchEvent extends Event {
  request: Request;
  respondWith(response: Promise<Response> | Response): void;
}

// Debug logging flag - set to true to enable comprehensive logging
const DEBUG_LOGGING = true;

// Development mode flag - injected by Vite
const DEV_MODE = typeof __DEV_MODE__ !== 'undefined' ? __DEV_MODE__ : false;
const DEV_SERVER_URL = 'http://localhost:3000';

type TonkState =
  | { status: 'uninitialized' }
  | {
      status: 'loading';
      promise: Promise<{ tonk: TonkCore; manifest: Manifest }>;
    }
  | { status: 'ready'; tonk: TonkCore; manifest: Manifest }
  | { status: 'failed'; error: Error };

let tonkState: TonkState = { status: 'uninitialized' };

// Helper to get tonk instance when ready
function getTonk(): { tonk: TonkCore; manifest: Manifest } | null {
  return tonkState.status === 'ready' ? tonkState : null;
}

// Logger utility
function log(level, message, data) {
  if (!DEBUG_LOGGING) return;

  const timestamp = new Date().toISOString();
  const prefix = `[VFS Service Worker ${timestamp}] ${level.toUpperCase()}:`;

  if (data !== undefined) {
    console[level](prefix, message, data);
  } else {
    console[level](prefix, message);
  }
}

// Worker state for file watchers
const watchers = new Map();

// Helper to post messages back to main thread
async function postResponse(response) {
  log('info', 'Posting response to main thread', {
    type: response.type,
    success: 'success' in response ? response.success : 'N/A',
  });

  // Get all clients and post message to each
  const clients = await (self as any).clients.matchAll();
  clients.forEach(client => {
    client.postMessage(response);
  });
}

// Check if development server is available
async function isDevServerAvailable() {
  if (!DEV_MODE) return false;

  try {
    await fetch(DEV_SERVER_URL + '/', {
      method: 'HEAD',
      mode: 'no-cors',
    });
    return true;
  } catch (error) {
    log('info', 'Dev server not available', { error: error.message });
    return false;
  }
}

// Proxy request to development server
async function proxyToDevServer(pathname) {
  try {
    const devUrl = DEV_SERVER_URL + pathname;
    log('info', 'Proxying to dev server', { pathname, devUrl });

    const response = await fetch(devUrl);
    if (response.ok) {
      return response;
    }

    log('info', 'Dev server returned non-OK status', {
      status: response.status,
      pathname,
    });
    return null;
  } catch (error) {
    log('info', 'Failed to proxy to dev server', {
      pathname,
      error: error.message,
    });
    return null;
  }
}

async function loadTonk(): Promise<{ tonk: TonkCore; manifest: Manifest }> {
  // TODO: have some way to pull the url here dynamically
  // Initialize WASM with the remote path
  await initializeTonk({ wasmPath: 'http://localhost:8081/tonk_core_bg.wasm' });

  // Fetch the manifest data
  const manifestResponse = await fetch('http://localhost:8081/.manifest.tonk');
  const manifestBytes = new Uint8Array(await manifestResponse.arrayBuffer());

  // NOTE:
  // This was going to be used for the entrypoint (to pull files out, but not doing that anymore)
  // we still might want this manifest to configure networks
  const bundle = await Bundle.fromBytes(manifestBytes);
  const manifest = await bundle.getManifest();

  const tonk = await TonkCore.fromBytes(manifestBytes);
  await tonk.connectWebsocket('ws://localhost:8081');
  return { tonk, manifest };
}

try {
  console.log('Before registration');
  const promise = loadTonk();
  tonkState = { status: 'loading', promise };

  promise
    .then(async ({ tonk, manifest }) => {
      tonkState = { status: 'ready', tonk, manifest };
      console.log('Tonk core service worker initialized');
      // Signal to main thread that worker is ready for VFS operations
      const clients = await (self as any).clients.matchAll();
      clients.forEach(client => {
        client.postMessage({ type: 'ready' });
      });
    })
    .catch(async error => {
      tonkState = { status: 'failed', error };
      console.log(`tonk error when initialising: `, error);
      // Still signal ready so the main thread knows we're operational (just without VFS)
      const clients = await (self as any).clients.matchAll();
      clients.forEach(client => {
        client.postMessage({ type: 'ready', tonkAvailable: false });
      });
    });
} catch (error) {
  console.log(`tonk error when initialising: `, error);
  tonkState = { status: 'failed', error };
}

self.addEventListener('install', () => {
  console.log('🔧 SERVICE WORKER: Installing SW');
  (self as any).skipWaiting();
});

self.addEventListener('activate', async () => {
  console.log('🚀 SERVICE WORKER: Activating service worker.');
  (self as any).clients.claim();
});

const targetToResponse = async (
  target: any,
  path?: string
): Promise<Response> => {
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

const determinePath = (url: URL, referrer?: string): string => {
  const serviceWorkerPath = self.location.pathname; // Path where the SW is registered
  const registrationScope =
    serviceWorkerPath.split('/').slice(0, -1).join('/') + '/'; // Base scope

  const requestPath = new URL(url).pathname;

  // Get the path relative to the service worker's registration scope
  let relativePath = requestPath.startsWith(registrationScope)
    ? requestPath.slice(registrationScope.length)
    : requestPath;

  // Handle referrer-based relative path resolution
  if (referrer) {
    try {
      const referrerUrl = new URL(referrer);
      if (referrerUrl.origin === url.origin) {
        const referrerPath = referrerUrl.pathname;

        // Get referrer path relative to registration scope
        const referrerRelativePath = referrerPath.startsWith(registrationScope)
          ? referrerPath.slice(registrationScope.length)
          : referrerPath;

        // Check if this request should be resolved relative to the referrer directory
        // This handles cases like: at /react/ requesting assets/vendor.js -> should go to /react/assets/vendor.js
        const referrerDir = referrerRelativePath
          .split('/')
          .slice(0, -1)
          .join('/');

        // If referrer is in a subdirectory and request doesn't start with that subdirectory
        if (
          referrerDir &&
          relativePath &&
          !relativePath.startsWith(referrerDir + '/')
        ) {
          relativePath = referrerDir + '/' + relativePath;
        }
      }
    } catch (e) {
      console.log('Failed to parse referrer URL:', e);
    }
  }

  // Special case for expected web server behavior
  const candidatePath = relativePath.split('/');
  if (candidatePath[candidatePath.length - 1] === '') {
    candidatePath[candidatePath.length - 1] = 'index.html';
  }

  return candidatePath.join('/');
};

(self as any).addEventListener('fetch', (event: FetchEvent) => {
  const url = new URL(event.request.url);
  const referrer = event.request.referrer;

  if (url.origin === location.origin) {
    event.respondWith(
      (async () => {
        const path = determinePath(url, referrer);
        const tonkInstance = getTonk();
        if (!tonkInstance) {
          throw new Error('Tonk not initialized');
        }
        try {
          const target = await tonkInstance.tonk.readFile(`/app/${path}`);
          return targetToResponse(target, path);
        } catch (e) {
          log('error', 'failed to fetch file', e);
        }

        const directories = await tonkInstance.tonk.listDirectory('/app');
        return new Response(
          `App path "${path}" not found. directories available: ${JSON.stringify(
            directories
          )}. Make sure the app is properly loaded.`,
          {
            status: 404,
            headers: { 'Content-Type': 'text/plain' },
          }
        );
      })()
    );
  }
});

async function handleMessage(message: VFSWorkerMessage) {
  log('info', 'Received message', {
    type: message.type,
    id: 'id' in message ? message.id : 'N/A',
  });

  if (tonkState.status !== 'ready' && message.type !== 'init') {
    log('error', 'Operation attempted before VFS initialization', {
      type: message.type,
      status: tonkState.status,
    });
    if ('id' in message) {
      postResponse({
        type: message.type,
        id: message.id,
        success: false,
        error: 'VFS not initialized',
      });
    }
    return;
  }

  switch (message.type) {
    case 'readFile':
      log('info', 'Reading file', { path: message.path, id: message.id });
      try {
        const { tonk } = getTonk()!;
        const content = await tonk.readFile(message.path);
        log('info', 'File read successfully', {
          path: message.path,
          contentType: typeof content,
          contentConstructor: content?.constructor?.name,
          isString: typeof content === 'string',
        });

        postResponse({
          type: 'readFile',
          id: message.id,
          success: true,
          data: { content: content.content, bytes: content.bytes },
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
        contentLength: message.content.length,
      });
      try {
        const { tonk } = getTonk()!;
        if (message.create) {
          log('info', 'Creating new file', { path: message.path });
          if (message.bytes) {
            await tonk.createFileWithBytes(
              message.path,
              JSON.parse(message.content),
              message.bytes
            );
          } else {
            await tonk.createFile(message.path, JSON.parse(message.content));
          }
        } else {
          log('info', 'Updating existing file', { path: message.path });
          await tonk.updateFile(message.path, JSON.parse(message.content));
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
        const watcher = await tonk.watchFile(
          message.path,
          (rawContent: { content: string }) => {
            log('info', 'File change detected', {
              watchId: message.id,
              path: message.path,
            });

            postResponse({
              type: 'fileChanged',
              watchId: message.id,
              content: JSON.parse(rawContent.content),
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
  }
}

// Listen for messages from main thread (for VFS operations)
self.addEventListener('message', async event => {
  try {
    await handleMessage(event.data);
  } catch (error) {
    log('error', 'Error handling message', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
});

// Worker startup
log('info', 'VFS Service Worker started', { debugLogging: DEBUG_LOGGING });

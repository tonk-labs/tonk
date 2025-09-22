/* eslint-env serviceworker */
/* global self, console, fetch, atob, btoa, caches, clients, location, URL, Response */
import { TonkCore, initializeTonk } from "@tonk/core/slim";
import mime from 'mime';

const CACHE_NAME = 'v6';

// Debug logging flag - set to true to enable comprehensive logging
const DEBUG_LOGGING = true;

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
function postResponse(response) {
  log('info', 'Posting response to main thread', {
    type: response.type,
    success: 'success' in response ? response.success : 'N/A',
  });
  self.postMessage(response);
}

async function loadTonk() {
  // Initialize WASM with the remote path
  await initializeTonk({ wasmPath: 'http://localhost:8081/tonk_core_bg.wasm' });

  // Fetch the manifest data
  const manifestResponse = await fetch('http://localhost:8081/.manifest.tonk');
  const manifestBytes = await manifestResponse.arrayBuffer();

  const tonk = await TonkCore.fromBytes(manifestBytes);
  await tonk.connectWebsocket('ws://localhost:8081');
  self.tonk = tonk;
  return tonk;
}

// Handle VFS file operations via messages
async function handleMessage(message) {
  log('info', 'Received message', {
    type: message.type,
    id: 'id' in message ? message.id : 'N/A',
  });

  if (!self.tonk && message.type !== 'init') {
    log('error', 'Operation attempted before VFS initialization', {
      type: message.type,
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
        const content = await self.tonk.readFile(message.path);
        log('info', 'File read successfully', {
          path: message.path,
          contentType: typeof content,
          contentConstructor: content?.constructor?.name,
          isString: typeof content === 'string',
        });

        // Extract content from Map and decode if needed
        let stringContent;
        if (content instanceof Map && content.has('content')) {
          const rawContent = content.get('content');
          if (typeof rawContent === 'string') {
            // Clean and decode base64 content
            try {
              // Remove any whitespace and non-base64 characters
              const cleanedContent = rawContent.replace(/[^A-Za-z0-9+/=]/g, '');

              // Ensure proper padding
              const paddedContent =
                cleanedContent +
                '==='.slice(0, (4 - (cleanedContent.length % 4)) % 4);

              stringContent = atob(paddedContent);
              log('info', 'Decoded base64 content', {
                originalLength: rawContent.length,
                cleanedLength: cleanedContent.length,
                decodedLength: stringContent.length,
              });
            } catch (error) {
              // If still not valid base64, use as-is
              log('warn', 'Failed to decode base64, using raw content', {
                error: error instanceof Error ? error.message : String(error),
                contentPreview: rawContent.substring(0, 50) + '...',
              });
              stringContent = rawContent;
            }
          } else {
            stringContent = String(rawContent);
          }
        } else if (typeof content === 'object' && content.content) {
          // Handle case where content is already parsed JSON
          const rawContent = content.content;
          try {
            stringContent = atob(rawContent.replace(/[^A-Za-z0-9+/=]/g, ''));
          } catch (error) {
            stringContent = rawContent;
          }
        } else {
          stringContent =
            typeof content === 'string' ? content : String(content);
        }

        log('info', 'Sending file content response', {
          contentLength: stringContent.length,
          preview: stringContent.substring(0, 100),
        });

        postResponse({
          type: 'readFile',
          id: message.id,
          success: true,
          data: stringContent,
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
        // Encode content as base64 for TonkCore
        const encodedContent = btoa(message.content);
        log('info', 'Encoded content for storage', {
          originalLength: message.content.length,
          encodedLength: encodedContent.length,
        });

        if (message.create) {
          log('info', 'Creating new file', { path: message.path });
          await self.tonk.createFile(message.path, encodedContent);
        } else {
          log('info', 'Updating existing file', { path: message.path });
          await self.tonk.updateFile(message.path, encodedContent);
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
        await self.tonk.deleteFile(message.path);
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
        const files = await self.tonk.listDirectory(message.path);
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
        const exists = await self.tonk.exists(message.path);
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
        const watcher = await self.tonk.watchFile(
          message.path,
          (rawContent) => {
            log('info', 'File change detected', {
              watchId: message.id,
              path: message.path,
            });

            const base64 = JSON.parse(rawContent).content;
            const decodedContent = atob(base64.replace(/[^A-Za-z0-9+/=]/g, ''));

            postResponse({
              type: 'fileChanged',
              watchId: message.id,
              content: decodedContent,
            });
          }
        );
        watchers.set(message.id, watcher);
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

try {
  console.log('Before registration');
  self.tonkPromise = loadTonk();
  self.tonkPromise.then(r => {
    self.tonk = r;
    console.log('Tonk core service worker initialized');
    // Signal to main thread that worker is ready for VFS operations
    self.postMessage({ type: 'ready' });
  });
} catch (error) {
  console.log(`tonk error when initialising: `, error); 
}

async function clearOldCaches() {
  const cacheWhitelist = [CACHE_NAME];
  const cacheNames = await caches.keys();
  const deletePromises = cacheNames.map(cacheName => {
    if (!cacheWhitelist.includes(cacheName)) {
      return caches.delete(cacheName);
    }
  });
  await Promise.all(deletePromises);
}

self.addEventListener('install', _event => {
  console.log('Installing SW');
  self.skipWaiting();
});

self.addEventListener('activate', async _event => {
  console.log('Activating service worker.');
  await clearOldCaches();
  clients.claim();
});

const targetToResponse = async (target, path) => {
  if (target.mimeType) {
    return new Response(target.content, {
      headers: { 'Content-Type': target.mimeType },
    });
  }

  if (typeof target === 'string') {
    try {
      const contentType = mime.getType(path) || 'application/octet-stream';

      console.log(`i think this is: ${contentType}`);

      return new Response(target, {
        headers: { 'Content-Type': contentType },
      });
    } catch (e) {
      // If fails, treat as plain text
      return new Response(target, {
        headers: { 'Content-Type': 'text/plain' },
      });
    }
  }

  return new Response(JSON.stringify(target), {
    headers: { 'Content-Type': 'application/json' },
  });
};

self.addEventListener('fetch', async event => {
  const url = new URL(event.request.url);
  console.log("response requested");

  if (url.origin === location.origin) {
    event.respondWith(
      (async () => {
        // Init tonk!
        try {
          await self.tonk.exists("/test");
          console.log("tonk is acting normally");
        } catch (error) {
          console.log("error when using tonk: ", error);
          try {
            console.log('Before registration');
            self.tonk = await loadTonk();
            console.log('Tonk core service worker initialized');
          } catch (error) {
            console.log(`tonk error when initialising: `, error); 
          }
        }
        let pathname = url.pathname;
        console.log('Service worker handling:', pathname);

        // Check if we're serving from an app context by looking at the referrer
        const referrer = event.request.referrer;
        let appContext = null;

        if (referrer) {
          const referrerUrl = new URL(referrer);
          const referrerPath = referrerUrl.pathname;
          // Extract app name if referrer is from /{app-name}/
          const appMatch = referrerPath.match(/^\/([^\/]+)\//);
          if (appMatch) {
            appContext = appMatch[1];
            if (appContext.includes('test-wasm')) {
              appContext.replace('test-wasm-2/', '');
              console.log("old legacy stuff here???");
            }
          }
        }

        // Handle root path - show app browser
        if (pathname === '/') {
          //pathname = '/index.html';

          let target; 
          try {
            target = await self.tonk.readFile('/app/index.html');
            console.log(`read ${'/app/index.html'} successfully!`);
          } catch (error) {
            console.log(`error inside asset branch trying to get ${'/app/index.html'}: ${error}`);
            console.log(error);
            target = null; 
          }
          console.log(target);

          if (target) {
            return targetToResponse(JSON.parse(target.content), '/app/index.html');
          }

          // For the app browser itself, try to fetch from cache or network
          const response = await caches.match(event.request);
          if (response) {
            return response;
          }
          return fetch(event.request);
        }

        // Handle app paths: /{app-name}/... or just /{app-name}
        // Check if this looks like an app path (not a static asset)
        const appPathMatch = pathname.match(/^\/([^\/]+)(?:\/(.*))?$/);
        if (appPathMatch && !pathname.startsWith('/assets/') && !pathname.includes('.js') && !pathname.includes('.css') && !pathname.includes('.wasm') && !pathname.includes('.svg')) {
          const [, appName, resourcePath = ''] = appPathMatch;
          let vfsPath = `/app/${appName}/${resourcePath}`;

          // If path ends with / or has no extension, append index.html
          if (vfsPath.endsWith('/') || (!vfsPath.includes('.', vfsPath.lastIndexOf('/')) && resourcePath !== '')) {
            if (!vfsPath.endsWith('/')) {
              vfsPath += '/';
            }
            vfsPath += 'index.html';
          } else if (resourcePath === '') {
            // Handle /{app-name} -> /app/{app-name}/index.html
            vfsPath = `/app/${appName}/index.html`;
          }
          
          console.log(`Mapping ${pathname} to VFS path: ${vfsPath}`);
          let target;
          try {
            target = await self.tonk.readFile(vfsPath);
            console.log(`read ${vfsPath} successfully!`);
          } catch (error) {
            console.log(`error inside app branch trying to get ${pathname}: ${error}`);

            // try defaulting to the home page if path to current page is not found, but restrict to html pages only
            if (vfsPath.includes('.html')) {
              console.log("rerouting to app/index.html...");
              try {
                target = await self.tonk.readFile('/app/index.html');
                console.log(`read /app/index.html successfully!`);
              } catch (error) {
                console.log(error);
                target = null; 
              }
            }
          }

          if (!target) {
            const directories = await self.tonk.listDirectory('/app');
            console.log(directories);
            return new Response(
              `App path "${pathname}" (mapped to "${vfsPath}") not found. directories available: ${JSON.stringify(directories)}. Make sure the app is properly loaded. error: ${error}`,
              {
                status: 404,
                headers: { 'Content-Type': 'text/plain' },
              }
            );
          }
          console.log(target);

          return targetToResponse(JSON.parse(target.content), vfsPath);
        }

        // Handle assets and other paths when in a specific subset of app context 
        // e.g. /app/test-wasm/index.html 
        if (appContext && !pathname.startsWith('/app/')) {
          // Rewrite the path to include the app context in VFS
          const rewrittenPath = `/app/${appContext}${pathname}`;
          console.log(`Rewriting ${pathname} to ${rewrittenPath}`);
          let target; 
          try {
            target = await self.tonk.readFile(rewrittenPath);
            console.log(`read ${rewrittenPath} successfully!`);
          } catch (error) {
            console.log(`error inside asset branch trying to get ${pathname}: ${error}`);
            console.log(error);
            target = null; 
          }
          console.log(target);

          if (target) {
            return targetToResponse(JSON.parse(target.content), rewrittenPath);
          }
        }

        // Handle assets and other paths when outside of app context
        if (!appContext && !(!pathname.includes('.js') && !pathname.includes('.css') && !pathname.includes('.wasm') && !pathname.includes('.svg') && !pathname.includes('.ico'))) {
          // Rewrite the path to include the app context in VFS
          const rewrittenPath = `/app${pathname}`;
          console.log(`Rewriting ${pathname} to ${rewrittenPath}`);

          let target; 
          try {
            target = await self.tonk.readFile(rewrittenPath);
            console.log(`read ${rewrittenPath} successfully!`);
          } catch (error) {
            console.log(`error inside asset branch (outside app context) trying to get ${pathname}: ${error}`);
            console.log(error);
            target = null; 
          }
          console.log(target);

          if (target) {
            return targetToResponse(JSON.parse(target.content), rewrittenPath);
          }
        }

        // Handle other static assets (fallback to network/cache)
        const response = await caches.match(event.request);
        if (response) {
          return response;
        }
        return fetch(event.request);
      })()
    );
  }
});

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

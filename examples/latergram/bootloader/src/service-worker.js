import { TonkCore, initializeTonk } from "@tonk-local/index-slim.js";

const CACHE_NAME = 'v6';

async function loadTonk() {
  // Initialize WASM with the remote path
  await initializeTonk({ wasmPath: 'http://localhost:8081/tonk_core_bg.wasm' });

  // Fetch the manifest data
  const manifestResponse = await fetch('http://localhost:8081/.manifest.tonk');
  const manifestBytes = await manifestResponse.arrayBuffer();

  const tonk = await TonkCore.fromBytes(new Uint8Array(manifestBytes));
  await tonk.connectWebsocket('ws://localhost:8081');
  self.tonk = tonk;
  return tonk;
}

console.log('Before registration');
const tonkPromise = loadTonk();
tonkPromise.then(r => {
  self.tonk = r;
  console.log('Tonk core service worker initialized');
});

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

const targetToResponse = async target => {
  if (target.mimeType) {
    return new Response(target.content, {
      headers: { 'Content-Type': target.mimeType },
    });
  }

  if (typeof target === 'string') {
    // Assume string content from VFS is base64-encoded
    try {
      const decodedContent = atob(target);
      // Try to determine content type from the content
      let contentType = 'text/plain';
      if (decodedContent.trim().startsWith('<!DOCTYPE html') || decodedContent.trim().startsWith('<html')) {
        contentType = 'text/html';
      } else if (decodedContent.trim().startsWith('{') || decodedContent.trim().startsWith('[')) {
        contentType = 'application/json';
      } else if (decodedContent.includes('function') || decodedContent.includes('const ') || decodedContent.includes('import ')) {
        contentType = 'application/javascript';
      } else if (decodedContent.includes('body {') || decodedContent.includes('@media')) {
        contentType = 'text/css';
      }

      return new Response(decodedContent, {
        headers: { 'Content-Type': contentType },
      });
    } catch (e) {
      // If base64 decode fails, treat as plain text
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
          }
        }

        // Handle root path - show app browser
        if (pathname === '/') {
          pathname = '/index.html';
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
        if (appPathMatch && !pathname.startsWith('/assets/') && !pathname.includes('.js') && !pathname.includes('.css') && !pathname.includes('.wasm')) {
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
          const target = await self.tonk.readFile(vfsPath);
          console.log(target);

          if (!target) {
            return new Response(
              `App path "${pathname}" (mapped to "${vfsPath}") not found. Make sure the app is properly loaded.`,
              {
                status: 404,
                headers: { 'Content-Type': 'text/plain' },
              }
            );
          }

          return targetToResponse(target);
        }

        // Handle assets and other paths when in app context
        if (appContext && !pathname.startsWith('/app/')) {
          // Rewrite the path to include the app context in VFS
          const rewrittenPath = `/app/${appContext}${pathname}`;
          console.log(`Rewriting ${pathname} to ${rewrittenPath}`);

          const target = await self.tonk.readFile(rewrittenPath);
          console.log(target);

          if (target) {
            return targetToResponse(target);
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

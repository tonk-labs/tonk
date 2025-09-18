import { TonkCore, initializeTonk } from "@tonk-local/index-slim.js";
import mime from 'mime';

const CACHE_NAME = 'v6';

async function loadTonk() {
  // Initialize WASM with the remote path
  await initializeTonk({ wasmPath: 'http://localhost:8081/tonk_core_bg.wasm' });

  // Fetch the manifest data
  const manifestResponse = await fetch('http://localhost:8081/.manifest.tonk');
  const manifestBytes = await manifestResponse.arrayBuffer();
  const manifestData = new Uint8Array([80, 75, 3, 4, 20, 0, 0, 0, 8, 0, 0, 0, 33, 0, 189, 99, 1, 115, 217, 0, 0, 0, 58, 1, 0, 0, 13, 0, 0, 0, 109, 97, 110, 105, 102, 101, 115, 116, 46, 106, 115, 111, 110, 77, 143, 49, 79, 195, 48, 16, 133, 247, 252, 138, 147, 87, 72, 228, 52, 137, 160, 222, 88, 144, 24, 138, 58, 52, 161, 8, 49, 84, 245, 33, 76, 154, 187, 234, 236, 134, 160, 42, 255, 29, 217, 64, 197, 248, 125, 126, 79, 126, 119, 206, 0, 212, 176, 35, 247, 134, 62, 116, 40, 222, 49, 41, 3, 229, 117, 244, 227, 133, 207, 25, 64, 10, 126, 176, 252, 61, 3, 168, 193, 81, 98, 157, 1, 204, 169, 34, 204, 225, 193, 42, 3, 202, 251, 119, 172, 199, 250, 185, 181, 237, 106, 187, 222, 158, 6, 234, 187, 169, 92, 57, 251, 180, 86, 41, 138, 20, 228, 235, 200, 142, 130, 87, 6, 94, 94, 147, 36, 12, 159, 44, 125, 43, 238, 159, 156, 30, 57, 96, 100, 58, 29, 14, 63, 166, 67, 178, 233, 235, 223, 101, 211, 134, 169, 191, 32, 128, 218, 11, 238, 2, 218, 187, 16, 183, 44, 244, 162, 201, 245, 50, 47, 111, 54, 101, 99, 154, 91, 83, 233, 162, 174, 170, 43, 173, 141, 214, 105, 77, 234, 224, 116, 100, 9, 104, 239, 133, 135, 88, 11, 76, 125, 190, 103, 65, 24, 117, 81, 22, 90, 165, 224, 28, 143, 205, 230, 111, 80, 75, 3, 4, 20, 0, 0, 0, 8, 0, 0, 0, 33, 0, 68, 12, 55, 184, 214, 0, 0, 0, 213, 0, 0, 0, 59, 0, 0, 0, 115, 116, 111, 114, 97, 103, 101, 47, 115, 115, 47, 104, 101, 52, 118, 52, 89, 85, 100, 85, 77, 88, 80, 88, 117, 109, 110, 107, 86, 120, 49, 77, 105, 100, 87, 80, 47, 115, 110, 97, 112, 115, 104, 111, 116, 47, 98, 117, 110, 100, 108, 101, 95, 101, 120, 112, 111, 114, 116, 107, 205, 247, 106, 182, 126, 42, 234, 204, 112, 138, 145, 81, 160, 187, 51, 224, 202, 92, 43, 247, 132, 29, 11, 62, 134, 207, 190, 34, 116, 149, 177, 187, 236, 199, 223, 222, 178, 72, 131, 63, 91, 111, 27, 68, 148, 221, 96, 14, 57, 236, 253, 97, 87, 153, 255, 251, 217, 18, 57, 231, 150, 77, 250, 35, 196, 206, 200, 196, 204, 36, 204, 172, 204, 228, 192, 236, 204, 20, 198, 196, 197, 200, 194, 196, 34, 106, 160, 200, 164, 204, 110, 194, 232, 196, 22, 198, 30, 46, 208, 192, 200, 196, 196, 192, 196, 88, 199, 192, 198, 196, 80, 199, 192, 88, 207, 192, 196, 206, 192, 194, 196, 192, 192, 194, 196, 92, 197, 145, 156, 145, 153, 147, 82, 148, 154, 199, 146, 151, 152, 155, 202, 85, 146, 153, 155, 90, 92, 146, 152, 91, 80, 204, 82, 82, 89, 144, 202, 158, 92, 148, 154, 88, 146, 154, 194, 145, 155, 159, 146, 153, 150, 153, 154, 194, 198, 80, 197, 86, 195, 88, 199, 204, 200, 86, 203, 196, 200, 192, 204, 88, 195, 32, 198, 96, 198, 148, 162, 159, 146, 89, 52, 127, 203, 140, 195, 83, 141, 33, 36, 27, 3, 35, 0, 80, 75, 1, 2, 20, 3, 20, 0, 0, 0, 8, 0, 0, 0, 33, 0, 189, 99, 1, 115, 217, 0, 0, 0, 58, 1, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 164, 129, 0, 0, 0, 0, 109, 97, 110, 105, 102, 101, 115, 116, 46, 106, 115, 111, 110, 80, 75, 1, 2, 20, 3, 20, 0, 0, 0, 8, 0, 0, 0, 33, 0, 68, 12, 55, 184, 214, 0, 0, 0, 213, 0, 0, 0, 59, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 164, 129, 4, 1, 0, 0, 115, 116, 111, 114, 97, 103, 101, 47, 115, 115, 47, 104, 101, 52, 118, 52, 89, 85, 100, 85, 77, 88, 80, 88, 117, 109, 110, 107, 86, 120, 49, 77, 105, 100, 87, 80, 47, 115, 110, 97, 112, 115, 104, 111, 116, 47, 98, 117, 110, 100, 108, 101, 95, 101, 120, 112, 111, 114, 116, 80, 75, 5, 6, 0, 0, 0, 0, 2, 0, 2, 0, 164, 0, 0, 0, 51, 2, 0, 0, 0, 0]);

  const tonk = await TonkCore.fromBytes(manifestData);
  await tonk.connectWebsocket('ws://localhost:8081');
  self.tonk = tonk;
  return tonk;
}

try {
  console.log('Before registration');
  self.tonkPromise = loadTonk();
  self.tonkPromise.then(r => {
    self.tonk = r;
    console.log('Tonk core service worker initialized');
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

/* const targetToResponse = async (target, path) => {
  if (target.mimeType) {
    return new Response(target.content, {
      headers: { 'Content-Type': target.mimeType },
    });
  }

  if (typeof target === 'string') {
    // Assume string content from VFS is base64-encoded
    try {
      const decodedContent = target;
      // Try to determine content type from the content
      let contentType = 'text/plain';
      if (decodedContent.trim().startsWith('<!DOCTYPE html') || decodedContent.trim().startsWith('<!doctype html') || decodedContent.trim().startsWith('<html')) {
        contentType = 'text/html';
      } else if (decodedContent.trim().startsWith('{') || decodedContent.trim().startsWith('[')) {
        contentType = 'application/json';
      } else if (decodedContent.includes('function') || decodedContent.includes('const ') || decodedContent.includes('import ')) {
        contentType = 'application/javascript';
      } else if (decodedContent.includes('body {') || decodedContent.includes('@media')) {
        contentType = 'text/css';
      }
      console.log(`i think this is: ${contentType}`);

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
}; */

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
            console.log("read file successfully");
          } catch (error) {
            console.log(error);
            target = null; 
          }
          //console.log(target);

          if (!target) {
            const directories = await self.tonk.listDirectory('/app');
            console.log(directories);
            return new Response(
              `App path "${pathname}" (mapped to "${vfsPath}") not found. directories: ${JSON.stringify(directories)}. Make sure the app is properly loaded.`,
              {
                status: 404,
                headers: { 'Content-Type': 'text/plain' },
              }
            );
          }
          console.log(target);

          return targetToResponse(JSON.parse(target.content), vfsPath);
        }

        // Handle assets and other paths when in app context
        if (appContext && !pathname.startsWith('/app/')) {
          // Rewrite the path to include the app context in VFS
          const rewrittenPath = `/app/${appContext}${pathname}`;
          console.log(`Rewriting ${pathname} to ${rewrittenPath}`);

          let target; 
          try {
            target = await self.tonk.readFile(rewrittenPath);
          } catch (error) {
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

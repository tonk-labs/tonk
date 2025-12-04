import { log } from './logging';
import { SERVE_LOCAL, DEV_SERVER_URL } from './constants';
import { getTonk, getAppSlug, setAppSlug } from './state';
import { persistAppSlug, clearPersistedState } from './persistence';
import type { FetchEvent } from './types';

// Convert VFS target to HTTP Response
export async function targetToResponse(target: {
  bytes?: string;
  content: unknown;
}): Promise<Response> {
  if (target.bytes) {
    // target.bytes is a base64 string, decode it to binary for Response
    const binaryString = atob(target.bytes);
    const bytes = new Uint8Array(binaryString.length);
    for (let i = 0; i < binaryString.length; i++) {
      bytes[i] = binaryString.charCodeAt(i);
    }
    return new Response(bytes, {
      headers: { 'Content-Type': (target.content as { mime: string }).mime },
    });
  } else {
    return new Response(JSON.stringify(target.content), {
      headers: { 'Content-Type': 'application/json' },
    });
  }
}

// Determine VFS path from URL
export function determinePath(url: URL): string {
  const appSlug = getAppSlug();

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
    ((self as unknown as ServiceWorkerGlobalScope).registration?.scope ??
      self.location.href) as string
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
      { pathSegments: [...pathSegments] }
    );
  } else {
    console.log(
      'determinePath - appSlug not present, using all segments as path',
      { pathSegments: [...pathSegments] }
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
}

// Check if URL is a Vite HMR asset
function isViteAsset(url: URL, appSlug: string | null): boolean {
  return (
    url.pathname.startsWith('/@vite') ||
    url.pathname.startsWith('/@react-refresh') ||
    url.pathname.startsWith('/src/') ||
    (appSlug !== null && url.pathname.startsWith(`/${appSlug}/@vite`)) ||
    (appSlug !== null && url.pathname.startsWith(`/${appSlug}/node_modules`)) ||
    (appSlug !== null && url.pathname.startsWith(`/${appSlug}/src/`)) ||
    url.pathname.startsWith('/node_modules') ||
    url.pathname.includes('__vite__') ||
    url.searchParams.has('t') // cache-busting params
  );
}

// Proxy request to dev server
async function proxyToDevServer(url: URL): Promise<Response> {
  const localDevUrl = `${DEV_SERVER_URL}${url.pathname}${url.search}`;

  log('info', 'Proxying request to dev server', {
    original: url.pathname,
    proxied: localDevUrl,
  });

  try {
    const response = await fetch(localDevUrl);
    log('info', 'Received response from dev server', {
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
      headers,
    });
  } catch (err) {
    log('error', 'Failed to fetch from dev server', {
      error: err instanceof Error ? err.message : String(err),
      url: url.pathname,
    });
    return new Response('Dev server unreachable', {
      status: 502,
      headers: { 'Content-Type': 'text/plain' },
    });
  }
}

// Serve file from VFS
async function serveFromVFS(url: URL): Promise<Response> {
  const appSlug = getAppSlug();
  const path = determinePath(url);

  log('info', 'Determined path for request', {
    path,
    originalUrl: url.href,
  });

  const tonkInstance = getTonk();
  log('info', 'Retrieved Tonk instance', {
    hasTonkInstance: !!tonkInstance,
  });

  if (!tonkInstance) {
    log('error', 'Tonk not initialized - cannot handle request', {
      path,
      url: url.href,
    });
    throw new Error('Tonk not initialized');
  }

  // Check if the file exists first
  const filePath = `/${path}`;
  const exists = await tonkInstance.tonk.exists(filePath);

  if (!exists) {
    console.warn(`🚨 File not found: ${filePath}, falling back to index.html`);
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

  log('info', 'File exists, attempting to read from Tonk', { filePath });
  const target = await tonkInstance.tonk.readFile(filePath);
  log('info', 'Successfully read file from Tonk', {
    filePath,
    hasContent: !!target.content,
    hasBytes: !!target.bytes,
  });

  return targetToResponse(target);
}

// Main fetch event handler
export function handleFetch(event: FetchEvent): void {
  const url = new URL(event.request.url);
  const appSlug = getAppSlug();

  console.log(
    '🔥 FETCH EVENT:',
    url.href,
    'Origin match:',
    url.origin === location.origin,
    'Pathname:',
    url.pathname
  );

  log('info', 'Fetch event received', {
    url: url.href,
    origin: url.origin,
    pathname: url.pathname,
    method: event.request.method,
    matchesOrigin: url.origin === location.origin,
  });

  // Check for WebSocket upgrade requests for Vite HMR
  const upgradeHeader = event.request.headers.get('upgrade');
  if (upgradeHeader && upgradeHeader.toLowerCase() === 'websocket') {
    log('info', '🔌 WS: WebSocket upgrade request detected - allowing through', {
      url: url.href,
    });
    // Don't intercept WebSocket connections - let them pass through
    return;
  }

  // Check if this is a request for the root hostname (should bypass VFS)
  const isRootRequest = url.pathname === '/' || url.pathname === '';
  if (isRootRequest && appSlug) {
    setAppSlug(null);
    // Persist the reset so it survives service worker restarts
    clearPersistedState().catch(err => {
      log('error', 'Failed to persist state reset', { error: err });
    });
  }

  // Dev mode: proxy to Vite server
  if (SERVE_LOCAL && appSlug && !isRootRequest) {
    if (isViteAsset(url, appSlug)) {
      log('info', 'Vite HMR asset detected, proxying to dev server', {
        url: url.href,
      });
      event.respondWith(proxyToDevServer(url));
      return;
    }

    log('info', 'TONK_SERVE_LOCAL enabled, proxying to local dev server');
    event.respondWith(proxyToDevServer(url));
    return;
  }

  // Production mode: serve from VFS
  if (appSlug && url.origin === location.origin && !isRootRequest) {
    log('info', 'Processing fetch request for same origin (non-root)');
    event.respondWith(
      serveFromVFS(url).catch(e => {
        log('error', 'Failed to fetch file from Tonk, falling back to original request', {
          error: e instanceof Error ? e.message : String(e),
          url: url.href,
        });

        // Instead of returning a 404, fall back to the original request
        log('info', 'Falling back to original fetch request', { url: url.href });
        return fetch(event.request);
      })
    );
  } else {
    log('info', 'Ignoring fetch request for different origin', {
      requestOrigin: url.origin,
      serviceWorkerOrigin: location.origin,
    });
  }
}

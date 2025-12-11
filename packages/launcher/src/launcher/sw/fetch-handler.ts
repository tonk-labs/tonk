import { persistAppSlug, persistBundleBytes } from './cache';
import { getActiveBundle, getInitializationPromise, getState } from './state';
import type { FetchEvent } from './types';
import { logger } from './utils/logging';
import { determinePath } from './utils/path';
import { targetToResponse } from './utils/response';

declare const TONK_SERVE_LOCAL: boolean;

export function handleFetch(event: FetchEvent): void {
  const url = new URL(event.request.url);
  const referrer = event.request.referrer;
  const state = getState();
  const activeBundle = getActiveBundle();
  const appSlug = activeBundle?.appSlug || null;

  logger.debug('Fetch event', {
    url: url.href,
    pathname: url.pathname,
    state: state.status,
    appSlug,
  });

  // Check for WebSocket upgrade requests for Vite HMR
  const upgradeHeader = event.request.headers.get('upgrade');
  if (upgradeHeader && upgradeHeader.toLowerCase() === 'websocket') {
    logger.debug('WebSocket upgrade request - passing through', {
      url: url.href,
    });
    return;
  }

  // Check if this is a request for the root hostname (should bypass VFS)
  const isRootRequest = url.pathname === '/' || url.pathname === '';
  if (isRootRequest && appSlug) {
    // Reset state when navigating to root
    // Persist the reset so it survives service worker restarts
    Promise.all([persistAppSlug(null), persistBundleBytes(null)]).catch((err) => {
      logger.error('Failed to persist state reset', { error: err });
    });
  }

  // Don't intercept requests for the runtime app's static files
  const isRuntimePath = url.pathname.startsWith('/app/');
  const hasBundleIdParam = url.searchParams.has('bundleId');

  // Known runtime static files that should never be served from VFS
  const runtimeStaticFiles = [
    '/app/index.html',
    '/app/main.js',
    '/app/main.css',
    '/app/service-worker-bundled.js',
  ];
  const isKnownRuntimeFile = runtimeStaticFiles.some((f) => url.pathname === f);

  // Also check for runtime font files (all common font formats)
  const fontExtensions = ['.otf', '.ttf', '.woff', '.woff2', '.eot'];
  const isRuntimeFont =
    url.pathname.startsWith('/app/') && fontExtensions.some((ext) => url.pathname.endsWith(ext));

  // Runtime static assets (HTML, CSS, JS, fonts) always pass through to network
  // These should NEVER go through VFS - they're part of the launcher shell
  if (isRuntimePath && (hasBundleIdParam || isKnownRuntimeFile || isRuntimeFont)) {
    logger.debug('Runtime static asset - passing through', {
      pathname: url.pathname,
    });
    return;
  }

  // VFS app requests: only pass through if appSlug !== 'app' (avoids path collision)
  // This handles the case where an app named 'app' would conflict with /app/* runtime paths
  if (isRuntimePath && appSlug !== 'app') {
    logger.debug('Runtime path with non-app slug - passing through', {
      pathname: url.pathname,
    });
    return;
  }

  // Handle TONK_SERVE_LOCAL mode (development with Vite HMR)
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
      logger.debug('Vite HMR asset - proxying to dev server', {
        pathname: url.pathname,
      });

      event.respondWith(
        (async () => {
          try {
            const localDevUrl = `http://localhost:4001${url.pathname}${url.search}`;
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
            logger.error('Failed to fetch Vite asset', {
              error: err instanceof Error ? err.message : String(err),
              pathname: url.pathname,
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

    logger.debug('TONK_SERVE_LOCAL - proxying to dev server', {
      pathname: url.pathname,
    });

    event.respondWith(
      (async () => {
        try {
          const localDevUrl = `http://localhost:4001${url.pathname}${url.search}`;
          const response = await fetch(localDevUrl);

          // Add no-cache headers
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
          logger.error('Failed to fetch from local dev server', {
            error: err instanceof Error ? err.message : String(err),
            pathname: url.pathname,
          });
          return new Response('Local dev server unreachable on port 4001', {
            status: 502,
            headers: { 'Content-Type': 'text/plain' },
          });
        }
      })()
    );
    return;
  }

  // Handle VFS requests (same origin, not root)
  if (appSlug && url.origin === location.origin && !isRootRequest) {
    logger.debug('Processing VFS request', {
      pathname: url.pathname,
      referrer,
    });

    event.respondWith(
      (async () => {
        try {
          const path = determinePath(url, appSlug);
          logger.debug('Resolved VFS path', { path, url: url.pathname });

          // Wait for auto-initialization to complete (with timeout)
          const initPromise = getInitializationPromise();
          if (initPromise && getState().status !== 'active') {
            logger.debug('Waiting for initialization', {
              status: getState().status,
            });
            try {
              await Promise.race([
                initPromise,
                new Promise((_, reject) =>
                  setTimeout(() => reject(new Error('Initialization timeout')), 15000)
                ),
              ]);
            } catch (waitError) {
              logger.warn('Initialization wait failed or timed out', {
                error: waitError instanceof Error ? waitError.message : String(waitError),
              });
            }
          }

          const tonkInstance = getActiveBundle();

          if (!tonkInstance) {
            logger.error('Tonk not initialized - cannot handle request', {
              status: getState().status,
              path,
            });
            throw new Error('Tonk not initialized');
          }

          // Check if the file exists first
          const filePath = `/${path}`;
          const exists = await tonkInstance.tonk.exists(filePath);

          if (!exists) {
            logger.debug('File not found, falling back to index.html', {
              path: filePath,
            });
            const indexPath = `/${appSlug}/index.html`;
            const target = await tonkInstance.tonk.readFile(indexPath);
            return targetToResponse(
              target as { bytes?: string; content: { mime?: string } | string }
            );
          }

          logger.debug('Serving file from VFS', { path: filePath });
          const target = await tonkInstance.tonk.readFile(filePath);

          const response = await targetToResponse(
            target as { bytes?: string; content: { mime?: string } | string }
          );
          return response;
        } catch (e) {
          logger.error('Failed to fetch from VFS', {
            error: e instanceof Error ? e.message : String(e),
            url: url.href,
            status: getState().status,
          });

          // Fall back to the original request
          logger.debug('Falling back to network request', { url: url.href });
          return fetch(event.request);
        }
      })()
    );
  } else {
    logger.debug('Ignoring fetch - different origin or root', {
      requestOrigin: url.origin,
      swOrigin: location.origin,
      isRoot: isRootRequest,
    });
  }
}

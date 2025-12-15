import { persistAppSlug, persistBundleBytes } from "./cache";
import {
  getActiveBundleState,
  getBundleState,
  getInitializationPromise,
} from "./state";
import type { FetchEvent } from "./types";
import { logger } from "./utils/logging";
import { determinePath, parseSpaceUrl } from "./utils/path";
import { targetToResponse } from "./utils/response";

declare const TONK_SERVE_LOCAL: boolean;

export function handleFetch(event: FetchEvent): void {
  const url = new URL(event.request.url);
  const referrer = event.request.referrer;

  logger.debug("Fetch event", {
    url: url.href,
    pathname: url.pathname,
  });

  // Check for WebSocket upgrade requests for Vite HMR
  const upgradeHeader = event.request.headers.get("upgrade");
  if (upgradeHeader && upgradeHeader.toLowerCase() === "websocket") {
    logger.debug("WebSocket upgrade request - passing through", {
      url: url.href,
    });
    return;
  }

  // Check if this is a request for the root or /space/ (should bypass VFS)
  const isRootRequest =
    url.pathname === "/" ||
    url.pathname === "" ||
    url.pathname === "/space/" ||
    url.pathname === "/space";
  if (isRootRequest) {
    // Reset state when navigating to root or /space/
    // Persist the reset so it survives service worker restarts
    Promise.all([persistAppSlug(null), persistBundleBytes(null)]).catch(
      (err) => {
        logger.error("Failed to persist state reset", { error: err });
      },
    );
    return;
  }

  // Don't intercept requests for the runtime app's static files
  // Runtime files are at /space/_runtime/, SW is at /space/
  // WASM is at /tonk_core_bg.wasm (outside /space/ scope, so not intercepted)
  const isRuntimePath = url.pathname.startsWith("/space/_runtime/");
  const hasBundleIdParam = url.searchParams.has("bundleId");

  // Known runtime static files that should never be served from VFS
  const runtimeStaticFiles = [
    "/space/_runtime/index.html",
    "/space/_runtime/main.js",
    "/space/_runtime/main.css",
    "/space/service-worker-bundled.js",
  ];
  const isKnownRuntimeFile = runtimeStaticFiles.some((f) => url.pathname === f);

  // Also check for runtime font files (all common font formats)
  const fontExtensions = [".otf", ".ttf", ".woff", ".woff2", ".eot"];
  const isRuntimeFont =
    url.pathname.startsWith("/space/_runtime/") &&
    fontExtensions.some((ext) => url.pathname.endsWith(ext));

  // Runtime static assets (HTML, CSS, JS, fonts, WASM) always pass through to network
  // These should NEVER go through VFS - they're part of the launcher shell
  if (
    isKnownRuntimeFile ||
    (isRuntimePath && (hasBundleIdParam || isRuntimeFont))
  ) {
    logger.debug("Runtime static asset - passing through", {
      pathname: url.pathname,
    });
    return;
  }

  // Parse the new URL structure: /space/<launcherBundleId>/<appSlug>/...
  const parsed = parseSpaceUrl(url.pathname);

  // _runtime is reserved for the launcher shell - pass through to network
  if (parsed?.launcherBundleId === "_runtime") {
    logger.debug("Reserved _runtime path - passing through", {
      pathname: url.pathname,
    });
    return;
  }

  // Handle TONK_SERVE_LOCAL mode (development with Vite HMR)
  if (TONK_SERVE_LOCAL) {
    const appSlug = parsed?.appSlug;

    // Check if this is a Vite dev asset that should be proxied to the dev server
    // These need to be proxied regardless of whether the URL matches /space/<bundleId>/<appSlug>/
    // because the controlled page loads assets with absolute paths like /src/main.tsx
    const isViteAsset =
      url.pathname.startsWith("/@vite") ||
      url.pathname.startsWith("/@react-refresh") ||
      url.pathname.startsWith("/@fs/") ||
      url.pathname.startsWith("/src/") ||
      url.pathname.startsWith("/node_modules") ||
      url.pathname.includes("__vite__") ||
      url.searchParams.has("t") || // cache-busting params
      (appSlug &&
        (url.pathname.startsWith(`/${appSlug}/@vite`) ||
          url.pathname.startsWith(`/${appSlug}/node_modules`) ||
          url.pathname.startsWith(`/${appSlug}/src/`)));

    if (isViteAsset) {
      logger.debug("Vite HMR asset - proxying to dev server", {
        pathname: url.pathname,
      });

      event.respondWith(
        (async () => {
          try {
            const localDevUrl = `http://localhost:4001${url.pathname}${url.search}`;
            const response = await fetch(localDevUrl);

            // Add no-cache headers to prevent browser caching during development
            const headers = new Headers(response.headers);
            headers.set("Cache-Control", "no-cache, no-store, must-revalidate");
            headers.set("Pragma", "no-cache");
            headers.set("Expires", "0");

            return new Response(response.body, {
              status: response.status,
              statusText: response.statusText,
              headers: headers,
            });
          } catch (err) {
            logger.error("Failed to fetch Vite asset", {
              error: err instanceof Error ? err.message : String(err),
              pathname: url.pathname,
            });
            return new Response("Vite dev server unreachable", {
              status: 502,
              headers: { "Content-Type": "text/plain" },
            });
          }
        })(),
      );
      return;
    }

    // For /space/<bundleId>/<appSlug>/ paths, also proxy to dev server
    if (appSlug && !isRootRequest) {
      logger.debug("TONK_SERVE_LOCAL - proxying to dev server", {
        pathname: url.pathname,
      });

      event.respondWith(
        (async () => {
          try {
            const localDevUrl = `http://localhost:4001${url.pathname}${url.search}`;
            const response = await fetch(localDevUrl);

            // Add no-cache headers
            const headers = new Headers(response.headers);
            headers.set("Cache-Control", "no-cache, no-store, must-revalidate");
            headers.set("Pragma", "no-cache");
            headers.set("Expires", "0");

            return new Response(response.body, {
              status: response.status,
              statusText: response.statusText,
              headers: headers,
            });
          } catch (err) {
            logger.error("Failed to fetch from local dev server", {
              error: err instanceof Error ? err.message : String(err),
              pathname: url.pathname,
            });
            return new Response("Local dev server unreachable on port 4001", {
              status: 502,
              headers: { "Content-Type": "text/plain" },
            });
          }
        })(),
      );
      return;
    }
  }

  // Handle VFS requests for /space/<launcherBundleId>/<appSlug>/... URLs
  if (parsed && url.origin === location.origin && !isRootRequest) {
    const { launcherBundleId, appSlug } = parsed;

    logger.debug("Processing VFS request", {
      pathname: url.pathname,
      launcherBundleId,
      appSlug,
      referrer,
    });

    event.respondWith(
      (async () => {
        try {
          // Determine the VFS path from the URL
          const path = determinePath(url, appSlug);
          logger.debug("Resolved VFS path", {
            path,
            url: url.pathname,
            launcherBundleId,
            appSlug,
          });

          // Wait for auto-initialization to complete (with timeout)
          const initPromise = getInitializationPromise();
          const bundleState = getBundleState(launcherBundleId);

          if (
            initPromise &&
            (!bundleState || bundleState.status !== "active")
          ) {
            logger.debug("Waiting for initialization", {
              launcherBundleId,
              status: bundleState?.status ?? "none",
            });
            try {
              await Promise.race([
                initPromise,
                new Promise((_, reject) =>
                  setTimeout(
                    () => reject(new Error("Initialization timeout")),
                    15000,
                  ),
                ),
              ]);
            } catch (waitError) {
              logger.warn("Initialization wait failed or timed out", {
                error:
                  waitError instanceof Error
                    ? waitError.message
                    : String(waitError),
              });
            }
          }

          const tonkInstance = getActiveBundleState(launcherBundleId);

          if (!tonkInstance) {
            logger.error(
              "Tonk not initialized for bundle - cannot handle request",
              {
                launcherBundleId,
                status: getBundleState(launcherBundleId)?.status ?? "none",
                path,
              },
            );
            throw new Error(`Bundle not initialized: ${launcherBundleId}`);
          }

          // Check if the file exists first
          const filePath = `/${path}`;
          const exists = await tonkInstance.tonk.exists(filePath);

          if (!exists) {
            logger.debug("File not found, falling back to index.html", {
              path: filePath,
            });
            const indexPath = `/${appSlug}/index.html`;
            const target = await tonkInstance.tonk.readFile(indexPath);
            return targetToResponse(
              target as { bytes?: string; content: { mime?: string } | string },
            );
          }

          logger.debug("Serving file from VFS", { path: filePath });
          const target = await tonkInstance.tonk.readFile(filePath);

          const response = await targetToResponse(
            target as { bytes?: string; content: { mime?: string } | string },
          );
          return response;
        } catch (e) {
          const errorMessage = e instanceof Error ? e.message : String(e);
          logger.error("Failed to fetch from VFS", {
            error: errorMessage,
            url: url.href,
            launcherBundleId,
          });

          // Return an error page instead of falling back to network
          // (network fallback would serve the launcher shell due to SPA routing)
          return new Response(
            `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Bundle Error</title>
  <style>
    body {
      font-family: system-ui, -apple-system, sans-serif;
      padding: 2rem;
      max-width: 600px;
      margin: 0 auto;
      background: #f5f5f5;
    }
    .error-container {
      background: white;
      border-radius: 8px;
      padding: 2rem;
      box-shadow: 0 2px 8px rgba(0,0,0,0.1);
    }
    h1 { color: #e53e3e; margin-top: 0; }
    .details { 
      background: #f7fafc; 
      padding: 1rem; 
      border-radius: 4px; 
      font-family: monospace;
      font-size: 0.875rem;
      word-break: break-all;
    }
    .actions { margin-top: 1.5rem; }
    button {
      background: #3182ce;
      color: white;
      border: none;
      padding: 0.5rem 1rem;
      border-radius: 4px;
      cursor: pointer;
      font-size: 1rem;
    }
    button:hover { background: #2c5282; }
  </style>
</head>
<body>
  <div class="error-container">
    <h1>Failed to Load Bundle</h1>
    <p>The application could not be loaded from the bundle.</p>
    <div class="details">
      <strong>Bundle ID:</strong> ${launcherBundleId}<br>
      <strong>Path:</strong> ${url.pathname}<br>
      <strong>Error:</strong> ${errorMessage}
    </div>
    <div class="actions">
      <button onclick="window.location.reload()">Retry</button>
    </div>
  </div>
</body>
</html>`,
            {
              status: 500,
              headers: { "Content-Type": "text/html" },
            },
          );
        }
      })(),
    );
  } else {
    logger.debug(
      "Ignoring fetch - not a valid /space/<bundleId>/<appSlug>/ request",
      {
        requestOrigin: url.origin,
        swOrigin: location.origin,
        parsed: parsed ? "yes" : "no",
        isRoot: isRootRequest,
      },
    );
  }
}

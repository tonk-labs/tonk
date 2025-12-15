/* eslint-env serviceworker */

import "./types"; // Import for global type declarations
import { handleFetch } from "./fetch-handler";
import { handleMessage } from "./message-handlers";
import { setInitializationPromise } from "./state";
import { autoInitializeFromCache } from "./tonk-lifecycle";
import type { FetchEvent } from "./types";
import { logger } from "./utils/logging";

declare const __SW_BUILD_TIME__: string;
declare const __SW_VERSION__: string;

const swSelf = self as unknown as ServiceWorkerGlobalScope;

// Service worker startup logging - always show version info
logger.info("Service worker starting", {
  version: __SW_VERSION__,
  buildTime: __SW_BUILD_TIME__,
  location: self.location.href,
});

// Start auto-initialization (non-blocking but tracked)
logger.debug("Checking for cached state");
setInitializationPromise(autoInitializeFromCache());

// Install event
self.addEventListener("install", (_event) => {
  logger.info("Service worker installing");
  swSelf.skipWaiting();
  logger.debug("skipWaiting called");
});

// Activate event with proper event.waitUntil for Safari compatibility
self.addEventListener("activate", (event) => {
  logger.info("Service worker activating");

  (event as ExtendableEvent).waitUntil(
    (async () => {
      // Claim clients first to take control of the page
      await swSelf.clients.claim();
      logger.debug("Clients claimed");

      // Then notify all clients that service worker is ready
      const allClients = await swSelf.clients.matchAll();

      allClients.forEach((client: Client) => {
        client.postMessage({ type: "ready", needsBundle: true });
      });

      logger.info("Service worker activated", {
        clientCount: allClients.length,
      });
    })(),
  );
});

// Message event
self.addEventListener("message", async (event: MessageEvent) => {
  logger.debug("Message received", {
    type: event.data?.type,
    hasSource: !!event.source,
  });

  // event.source is the Client that sent the message
  const sourceClient = event.source as Client | null;
  if (!sourceClient) {
    logger.error("Message received without source client", {
      type: event.data?.type,
    });
    return;
  }

  try {
    await handleMessage(event.data, sourceClient);
  } catch (error) {
    logger.error("Error handling message", {
      error: error instanceof Error ? error.message : String(error),
    });
  }
});

// Fetch event
// @ts-expect-error - FetchEvent type from our types.ts
self.addEventListener("fetch", (event: FetchEvent) => {
  handleFetch(event);
});

// Worker startup complete
logger.debug("VFS Service Worker initialized");

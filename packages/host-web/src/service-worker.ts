/* eslint-env serviceworker */
/* eslint-disable @typescript-eslint/no-non-null-assertion */

import { log } from './service-worker/logging';
import { SW_VERSION, SW_BUILD_TIME, DEBUG_LOGGING } from './service-worker/constants';
import { setTonkState } from './service-worker/state';
import { handleMessage } from './service-worker/message-handler';
import { handleFetch } from './service-worker/fetch-handler';
import { autoInitializeFromCache } from './service-worker/lifecycle';
import { postResponse } from './service-worker/message-utils';
import type { VFSWorkerMessage, FetchEvent } from './service-worker/types';

// Startup logging
console.log('🚀 SERVICE WORKER: Script loaded at', new Date().toISOString());
console.log('🔍 DEBUG_LOGGING enabled:', DEBUG_LOGGING);
console.log('🌐 Service worker location:', self.location.href);
console.log('🔧 SERVICE WORKER VERSION:', SW_VERSION);
console.log('🕐 SERVICE WORKER BUILD TIME:', SW_BUILD_TIME);

// Initialize state
log('info', 'Service worker starting, checking for cached state');
console.log('🚀 SERVICE WORKER: Starting initialization');
setTonkState({ status: 'uninitialized' });

// Start auto-initialization (non-blocking)
autoInitializeFromCache();

// Install handler
self.addEventListener('install', _event => {
  log('info', 'Service worker installing');
  console.log('🔧 SERVICE WORKER: Installing SW');
  (self as unknown as ServiceWorkerGlobalScope).skipWaiting();
  log('info', 'Service worker install completed, skipWaiting called');
});

// Activate handler with proper event.waitUntil for Safari compatibility
self.addEventListener('activate', event => {
  log('info', 'Service worker activating');
  console.log('🚀 SERVICE WORKER: Activating service worker.');

  // Use waitUntil to ensure Safari waits for async operations to complete
  (event as ExtendableEvent).waitUntil(
    (async () => {
      // Claim clients first to take control of the page
      await (self as unknown as ServiceWorkerGlobalScope).clients.claim();
      log('info', 'Service worker activation completed, clients claimed');

      // Then notify all clients that service worker is ready
      await postResponse({ type: 'ready', needsBundle: true });
      log(
        'info',
        'Service worker activated, ready message sent to all clients'
      );
      console.log('🚀 SERVICE WORKER: Activated and ready');
    })()
  );
});

// Fetch handler
(self as unknown as ServiceWorkerGlobalScope).addEventListener(
  'fetch',
  (event: Event) => {
    handleFetch(event as unknown as FetchEvent);
  }
);

// Message handler
self.addEventListener('message', async event => {
  log('info', 'Raw message event received', {
    eventType: event.type,
    messageType: (event as MessageEvent).data?.type,
    hasData: !!(event as MessageEvent).data,
  });
  try {
    await handleMessage((event as MessageEvent).data as VFSWorkerMessage);
  } catch (error) {
    log('error', 'Error handling message', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
});

// Worker startup complete
log('info', 'VFS Service Worker started', { debugLogging: DEBUG_LOGGING });

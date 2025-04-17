/// <reference lib="webworker" />

// This prevents TypeScript errors in service worker context
const sw = self as unknown as ServiceWorkerGlobalScope;

// Workbox injects manifest here
// @ts-ignore
const manifestEntries = self.__WB_MANIFEST;

const CACHE_NAME = "tinyfoot-app-v1";

// Define asset types to cache by extension
const CACHEABLE_EXTENSIONS = [
  ".html",
  ".css",
  ".js",
  ".json",
  ".png",
  ".jpg",
  ".jpeg",
  ".svg",
  ".ico",
  ".woff",
  ".woff2",
  ".ttf",
  ".mp3",
  ".wav",
  ".ogg",
  ".m4a",
  ".wasm",
];

// Install event
sw.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(CACHE_NAME).then((cache) => {
      return cache.addAll([
        "/",
        "/index.html",
        "/offline.html",
        "/manifest.json",
        "/favicon.ico",
        "/bundle.js",
        "/icons/icon-192x192.png",
        "/icons/icon-512x512.png",
        "/1234567890abc.module.wasm", // will get replaced with actual module on build
      ]);
    }),
  );
  sw.skipWaiting();
});

// Activate event
sw.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((cacheNames) => {
      return Promise.all(
        cacheNames
          .filter((cacheName) => cacheName !== CACHE_NAME)
          .map((cacheName) => caches.delete(cacheName)),
      );
    }),
  );
  sw.clients.claim();
});

sw.addEventListener("fetch", (event) => {
  event.respondWith(
    (async () => {
      const cachedResponse = await caches.match(event.request);
      if (cachedResponse) {
        // If this is a wasm file, ensure it has the correct MIME type
        if (event.request.url.endsWith(".wasm")) {
          // Create a new response with the correct MIME type
          return new Response(cachedResponse.body, {
            headers: {
              "Content-Type": "application/wasm",
              ...cachedResponse.headers,
            },
            status: cachedResponse.status,
            statusText: cachedResponse.statusText,
          });
        }
        return cachedResponse;
      }

      try {
        const response = await fetch(event.request);

        // Cache successful responses
        if (response.ok) {
          const url = new URL(event.request.url);
          if (CACHEABLE_EXTENSIONS.some((ext) => url.pathname.endsWith(ext))) {
            const responseToCache = response.clone();
            caches.open(CACHE_NAME).then((cache) => {
              cache.put(event.request, responseToCache);
            });
          }
        }

        return response;
      } catch (err) {
        const url = new URL(event.request.url);

        const pathname = url.pathname;
        const filename = pathname
          .substring(1 + pathname.lastIndexOf("/"))
          .split(/\#|\?/g)[0];

        // If we're navigating to a page and not requesting a specific asset file,
        // try to return the index page from cache
        if (
          url.origin === location.origin &&
          !CACHEABLE_EXTENSIONS.some((ext) => filename.endsWith(ext))
        ) {
          const cachedIndex = await caches.match("/");
          if (cachedIndex) return cachedIndex;

          // If index isn't cached, try the offline page
          const offlinePage = await caches.match("/offline.html");
          if (offlinePage) return offlinePage;
        }

        throw err;
      }
    })(),
  );
});

/// <reference lib="webworker" />

// This prevents TypeScript errors in service worker context
const sw = self as unknown as ServiceWorkerGlobalScope;

const CACHE_NAME = "tinyfoot-app-v1";
const ASSETS_TO_CACHE = [
  "/",
  "/index.html",
  "/bundle.js",
  "/manifest.json",
  "/offline.html",
  "/icons/icon-192x192.png",
  "/icons/icon-512x512.png",
];

// Install event
sw.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(CACHE_NAME).then((cache) => {
      return cache.addAll(ASSETS_TO_CACHE);
    })
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
          .map((cacheName) => caches.delete(cacheName))
      );
    })
  );
  sw.clients.claim();
});

// Fetch event
sw.addEventListener("fetch", (event) => {
  // Only handle GET requests
  if (event.request.method !== "GET") return;

  // Handle navigation requests
  if (event.request.mode === "navigate") {
    event.respondWith(
      fetch(event.request).catch(() => {
        return caches.match("/offline.html") as Promise<Response>;
      })
    );
    return;
  }

  // Cache first for static assets
  event.respondWith(
    caches.match(event.request).then((cachedResponse) => {
      if (cachedResponse) {
        return cachedResponse;
      }

      return fetch(event.request)
        .then((response) => {
          // Cache successful responses
          if (response.ok) {
            const clone = response.clone();
            caches.open(CACHE_NAME).then((cache) => {
              cache.put(event.request, clone);
            });
          }
          return response;
        })
        .catch(() => {
          // Return a fallback for images
          if (event.request.destination === "image") {
            return caches.match("/icons/icon-192x192.png") as Promise<Response>;
          }
          return new Response("Not available offline", {
            status: 503,
            statusText: "Service Unavailable",
          });
        });
    })
  );
});

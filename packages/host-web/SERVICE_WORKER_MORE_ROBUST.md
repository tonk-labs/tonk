# Service Worker More Robust Handling

## Problem

Arc browser exhibits inconsistent service worker behavior compared to Chrome Canary:
- Regular Arc: HEAD requests to `/latergram/` return 404 even after service worker reports "ready"
- Chrome Canary: Works correctly
- Arc Incognito: Works after "fiddling with it enough"

This suggests service worker persistence/caching issues rather than fetch interception timing.

## Root Cause

The incognito behavior indicates:
1. **Stale Service Worker Cache**: Arc uses cached SW versions with bugs/incorrect fetch handlers
2. **Multiple SW Versions**: Conflicting registrations interfere with each other  
3. **Registration Scope Issues**: Previous SW registrations cause conflicts

Incognito works because it provides a clean slate without cached SW state.

## Solutions

### 1. Force Update on Registration
```javascript
navigator.serviceWorker.register(serviceWorkerUrl, { 
  type: "module",
  updateViaCache: 'none'  // Bypass cache for SW script
})
.then(registration => {
  // Force check for updates
  registration.update();
})
```

### 2. Handle the `updatefound` Event
```javascript
navigator.serviceWorker.register(serviceWorkerUrl)
.then(registration => {
  registration.addEventListener('updatefound', () => {
    const newWorker = registration.installing;
    newWorker.addEventListener('statechange', () => {
      if (newWorker.state === 'installed' && navigator.serviceWorker.controller) {
        // New SW installed but old one still controlling
        // Option 1: Auto-reload
        window.location.reload();
        
        // Option 2: Prompt user
        if (confirm('New version available. Reload?')) {
          window.location.reload();
        }
      }
    });
  });
});
```

### 3. Clear Stale Registrations
```javascript
// Clear all existing registrations before registering new one
const registrations = await navigator.serviceWorker.getRegistrations();
await Promise.all(registrations.map(reg => reg.unregister()));

// Then register fresh
await navigator.serviceWorker.register(serviceWorkerUrl);
```

### 4. Version-based Cache Busting
```javascript
const version = Date.now(); // or app version
const serviceWorkerUrl = `./service-worker-bundled.js?v=${version}`;
```

### 5. In Service Worker - Handle Updates Gracefully
```javascript
self.addEventListener('install', event => {
  self.skipWaiting(); // Take control immediately
});

self.addEventListener('activate', event => {
  event.waitUntil(
    self.clients.claim().then(() => {
      // Clear old caches if needed
      return caches.keys().then(names => 
        Promise.all(names.map(name => caches.delete(name)))
      );
    })
  );
});
```

## Recommended Implementation

For the Arc browser issue, implement:

1. **Immediate fix**: Add `updateViaCache: 'none'` and force update check
2. **Robust solution**: Clear stale registrations before new registration
3. **Development aid**: Version-based cache busting during development

### Updated Registration Code
```javascript
// In index.html
(async () => {
  if ("serviceWorker" in navigator) {
    // Clear any stale registrations first
    const registrations = await navigator.serviceWorker.getRegistrations();
    await Promise.all(registrations.map(reg => reg.unregister()));
    
    // Version-based cache busting
    const version = Date.now();
    let serviceWorkerUrl = `./service-worker-bundled.js?v=${version}`;
    
    if (bundleParam) {
      serviceWorkerUrl += `&bundle=${encodeURIComponent(bundleParam)}`;
    }
    
    const registration = await navigator.serviceWorker.register(serviceWorkerUrl, { 
      type: "module",
      updateViaCache: 'none'
    });
    
    // Force update check
    await registration.update();
    
    // Rest of existing logic...
  }
})();
```

This should resolve the Arc browser inconsistency by ensuring clean service worker state on each page load.
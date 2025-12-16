# Gotchas and Known Issues

**Read this document first.** These issues waste hours of debugging time if you ignore them.

## Critical Issues (P0)

These block development until addressed.

### Runtime Changes Require Manual Rebuild

**Symptom:** You edit files in `src/runtime/`, but changes never appear.

**Cause:** The runtime app (`public/app/main.js`) is pre-built. The dev server ignores it.

**Fix:** Rebuild after every runtime change:

```bash
vite build -c vite.runtime.config.ts && cp -r dist-runtime/* public/app/
```

**Tip:** Add an alias to your shell:

```bash
alias rebuild-runtime="cd /path/to/launcher && vite build -c vite.runtime.config.ts && cp -r dist-runtime/* public/app/"
```

---

### Firefox Lacks Support

**Symptom:** "Service Worker registration failed" error.

**Cause:** Firefox lacks ES module support in service workers.

**Fix:** Use Chrome or Safari.

---

### Service Worker Caching

**Symptom:** Old service worker behavior persists after code changes.

**Cause:** Browsers cache service workers aggressively. A running SW refuses to update until all tabs close.

**Fix:** Unregister the SW in DevTools (Application → Service Workers) and hard refresh.

**Tip:** Enable "Update on reload" in DevTools during development.

---

### TONK_SERVE_LOCAL Confusion

**Symptom:** CORS errors referencing `localhost:4001`.

**Cause:** The service worker was built with `TONK_SERVE_LOCAL=true`, which proxies requests to a local dev server on port 4001. This mode supports developing Tonk *apps*, not the launcher itself.

**Fix:** Either:

1. Run your Tonk app dev server on port 4001, OR
2. Rebuild the SW without local serving:

```bash
TONK_SERVE_LOCAL=false vite build -c vite.sw.config.ts && cp dist-sw/service-worker-bundled.js public/app/
```

---

## High Priority Issues (P1)

Common development issues.

### Path Collision with "app" Slug

**Symptom:** Routing breaks when a bundle has the name "app".

**Cause:** The runtime lives at `/app/`. A bundle named "app" creates a path conflict.

**Fix:** Avoid naming bundles "app".

---

### WASM Path Resolution

**Symptom:** "Failed to fetch WASM" or 404 errors for `.wasm` files.

**Cause:** The WASM file is missing from `public/app/`.

**Fix:**

```bash
bun run setup-wasm
```

---

### Cross-Feature Import Violations

**Symptom:** Lint fails with "no-cross-feature-imports" error.

**Cause:** You imported from `launcher/` in `runtime/` code (or vice versa).

**Fix:** Move shared code to:
- `src/lib/` for utilities
- `src/components/` for UI components
- `src/hooks/` for shared hooks

---

### 120-Second Message Timeout

**Symptom:** Long operations fail with timeout errors.

**Cause:** Service worker messages timeout after 120 seconds.

**Fix:** Break long operations into smaller chunks, or implement progress updates.

---

## Medium Priority Issues (P2)

Behavioral quirks to know.

### Theme Does Not Propagate to iframe

**Symptom:** Dark mode works in the launcher but not in the running bundle.

**Cause:** The iframe is isolated. Theme changes require explicit messaging.

**Fix:** Bundles must listen for `theme-change` postMessage events from the parent.

---

### Bundle Already Loaded Check

**Symptom:** Reloading the same bundle seems to skip initialization.

**Cause:** The SW compares `manifest.rootId` and skips reload if unchanged.

**Fix:** Clear the SW cache or use a different bundle version.

---

### PathIndex Sync Delay

**Symptom:** Brief delay before app renders after bundle load.

**Cause:** The SW waits up to 1 second for CRDT PathIndex to sync from remote peers.

**Note:** This is intentional. The delay ensures the file listing is current.

---

### Service Worker Log Verbosity

**Symptom:** Minimal logs in service worker console.

**Cause:** Default log level minimizes output for performance.

**Fix:** In the SW console, run:

```javascript
self.SW_LOG_LEVEL = 'debug'
```

---

### No Automatic Bundle Cleanup

**Symptom:** IndexedDB storage grows over time.

**Cause:** Deleting bundles from the UI does not always clean up all data.

**Fix:** Delete old bundles manually. Consider implementing a storage quota check.

---

## Low Priority Issues (P3)

Edge cases.

### WebSocket Reconnection

**Symptom:** Console spam about reconnection attempts.

**Cause:** Aggressive reconnection (10 attempts with exponential backoff).

**Note:** This behavior is intentional for reliability. Consider reducing for mobile/battery scenarios.

---

### manifest.entrypoints Required

**Symptom:** Bundle import fails silently.

**Cause:** Bundles must have at least one entrypoint in their manifest.

**Fix:** Ensure bundles are properly built with entrypoints defined.

---

### Root Request Clears State

**Symptom:** Navigating to `/` clears the running bundle.

**Note:** This is intentional. It serves as the "exit" mechanism to return to the launcher.

---

## Quick Reference

### Service Worker Debugging

```javascript
// In SW console
self.SW_LOG_LEVEL = 'debug'  // Enable verbose logging

// Check current state
console.log(await caches.keys())  // See cached data
```

### Force Clean Slate

```bash
# 1. Unregister SW in DevTools
# 2. Clear site data: DevTools → Application → Clear site data
# 3. Hard refresh
# 4. Rebuild everything:
bun run build:runtime
bun run dev
```

### Common Rebuild Commands

```bash
# Runtime only
vite build -c vite.runtime.config.ts && cp -r dist-runtime/* public/app/

# Service worker only
vite build -c vite.sw.config.ts && cp dist-sw/service-worker-bundled.js public/app/

# Service worker (production mode)
TONK_SERVE_LOCAL=false vite build -c vite.sw.config.ts && cp dist-sw/service-worker-bundled.js public/app/

# Both runtime and SW
bun run build:runtime
```

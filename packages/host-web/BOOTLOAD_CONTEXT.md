# Bootload Context: Host-Web + Latergram + Server Architecture

This document explains the complex multi-component architecture used for loading and running Tonk applications, specifically the relationship between `host-web`, `latergram`, and the `latergram/server`.

## High-Level Architecture

The system consists of three interconnected components that work together to provide a dynamic, WASM-based application environment:

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   latergram     │───▶│ latergram/server│───▶│    host-web     │
│   (bundled app) │    │  (serves files) │    │ (tonk runtime)  │
└─────────────────┘    └─────────────────┘    └─────────────────┘
        │                       │                       │
        │                       │                       │
    .tonk bundle           WASM + Manifest          Service Worker
   (ZIP archive)          HTTP endpoints           + Tonk Runtime
```

## Component Breakdown

### 1. Latergram (Application)
- **Location**: `/examples/latergram/`
- **Purpose**: A React-based application that gets compiled and bundled into a `.tonk` file
- **Build Process**: 
  - TypeScript/React code gets compiled
  - Assets are processed and bundled
  - Everything is packaged into a `.tonk` file (ZIP archive format)
  - Contains: `manifest.json`, application files, and bundled assets

### 2. Latergram Server
- **Location**: `/examples/latergram/server/`
- **Purpose**: HTTP server that serves both the bundled application and WASM runtime files
- **Port**: `localhost:8081`
- **Key Endpoints**:
  - `/tonk_core_bg.wasm` - WASM binary for Tonk runtime (served from `packages/core-js/dist/`)
  - `/.manifest.tonk` - Application manifest and bundle data
  - WebSocket endpoint for real-time sync

### 3. Host-Web (Runtime Environment)
- **Location**: `/packages/host-web/`
- **Purpose**: Provides the runtime environment that loads and executes `.tonk` applications
- **Port**: `localhost:3000`
- **Architecture**: Uses a Service Worker to intercept requests and serve files from the virtual file system

## Detailed Flow

### 1. Application Bundling
```bash
# In latergram directory
pnpm bundle create --copy-to-server  # Creates latergram.tonk bundle and copies to server
```

### 2. Server Startup
```bash
# In latergram/server directory
npm run dev  # Starts server on port 8081
```
- Server loads the `.tonk` bundle using `BundleStorageAdapter`
- Serves WASM runtime from `packages/core-js/dist/tonk_core_bg.wasm`
- Provides WebSocket endpoint for real-time synchronization

### 3. Host-Web Initialization
```bash
# In host-web directory
npm run dev  # Starts development server on port 3000
```

### 4. Service Worker Bootload Process

When you visit `localhost:3000`, the following happens:

1. **Service Worker Registration**: Browser loads and registers the service worker
2. **WASM Initialization**: 
   - Service worker fetches WASM binary from `http://localhost:8081/tonk_core_bg.wasm`
   - Initializes Tonk runtime with matching JavaScript bindings
3. **Manifest Loading**:
   - Fetches application manifest from `http://localhost:8081/.manifest.tonk`
   - Loads bundle metadata and virtual file system structure
4. **WebSocket Connection**: Establishes real-time sync connection to `ws://localhost:8081`

### 5. Application Access and URL Structure

**IMPORTANT**: To access a bundled application, you must visit the project-specific URL:

```
localhost:3000/${project-name}/
```

For latergram, this means visiting: `localhost:3000/latergram/`

**Why this structure?**
- Applications are bundled with their project name as a namespace: `/app/${project-name}/`
- This enables **multi-app support** - a single bundle could contain multiple applications
- Each application is isolated within its own directory structure
- The URL path maps directly to the VFS structure

### 5a. Redirect Mechanism (404.html)

Host-web includes a clever redirect system to handle direct navigation to application URLs:

**The Problem**: When you directly visit `localhost:3000/latergram/`, the static file server returns a 404 because the service worker hasn't loaded yet.

**The Solution**: A two-step redirect process:

1. **404.html Intercept** (`/packages/host-web/src/404.html`):
   ```javascript
   // Saves the intended URL and redirects to root
   sessionStorage.setItem("redirectPath", window.location.href)
   window.location.href = "/"
   ```

2. **Service Worker Bootstrap** (`/packages/host-web/src/index.html`):
   ```javascript
   // After service worker loads, checks for saved redirect
   const redirectPath = sessionStorage.getItem("redirectPath")
   if (redirectPath) {
     sessionStorage.removeItem("redirectPath")
     window.location.href = redirectPath  // Back to /latergram/
   }
   ```

**Flow**:
1. User visits `localhost:3000/latergram/` → 404.html
2. 404.html saves `/latergram/` to sessionStorage, redirects to `/`
3. Root loads, registers service worker
4. Service worker takes control, index.html reads sessionStorage
5. Redirects back to `localhost:3000/latergram/` → now served by service worker

### 6. Request Interception and Path Mapping

Once initialized, the service worker intercepts all requests to `localhost:3000`:

```javascript
// Example: Request to localhost:3000/latergram/app.js
// 1. Service worker intercepts the request
// 2. Extracts path: "latergram/app.js"
// 3. Maps to VFS path: /app/latergram/app.js  
// 4. Reads file from Tonk VFS: tonk.readFile('/app/latergram/app.js')
// 5. Returns file content as HTTP response
```

**Bundle Structure:**
```
/app/
  └── latergram/           ← Project namespace
      ├── index.html       ← Main application entry
      ├── assets/
      │   ├── index.js     ← Bundled JavaScript
      │   └── styles.css   ← Bundled CSS
      └── manifest.json    ← Application metadata
```

## Key Technical Details

### WASM + JavaScript Binding Synchronization
- **Critical Issue**: WASM binary and JavaScript bindings must be from the same build
- **Problem**: Browser caching can serve stale WASM while JS bindings are updated
- **Solution**: Cache-busting query parameters (`?t=${timestamp}`) force fresh WASM fetch
- **Error Pattern**: `__wbindgen_closure_wrapper[NUMBER]` import mismatches indicate version skew

### Virtual File System (VFS)
- Powered by Tonk WASM runtime
- Files stored in CRDT (Conflict-free Replicated Data Type) format
- Real-time synchronization via WebSocket
- All file operations go through `tonkCore.readFile()`, `tonkCore.writeFile()`, etc.

### Bundle Format (.tonk files)
- ZIP archive containing:
  - `manifest.json` - Application metadata and configuration
  - Application files organized in directory structure
  - Assets and compiled code
- Loaded by `BundleStorageAdapter` on server side
- Provides initial state for virtual file system

## Development Workflow

### Making Changes to the Application
1. Edit files in `/examples/latergram/src/`
2. Run `pnpm bundle create --copy-to-server` to create new `.tonk` bundle and copy to server
3. Server automatically picks up the new bundle
4. Refresh browser to load updated application

### Debugging Issues
1. **Service Worker Problems**: Check browser DevTools → Application → Service Workers
2. **WASM Issues**: Look for closure wrapper import errors in console
3. **VFS Issues**: Check if files exist in bundle and are being served correctly
4. **Network Issues**: Verify server is running on port 8081 and serving endpoints

### Common Gotchas
- **Wrong URL**: Must visit `localhost:3000/${project-name}/` not just `localhost:3000/`
- **Browser Caching**: WASM files are aggressively cached - use hard refresh or cache-busting
- **Service Worker Persistence**: May need to unregister and re-register after major changes
- **Build Dependencies**: Changes to `packages/core-js` require rebuilding both core and host-web
- **Port Conflicts**: Ensure both servers (3000 and 8081) are available
- **Environment Variables**: Ensure `.env` file exists with required keys before building bundle

## File System Paths

### Build Artifacts
- Latergram bundle: `/examples/latergram/latergram.tonk`
- WASM runtime: `/packages/core-js/dist/tonk_core_bg.wasm`
- Service worker: `/packages/host-web/dist/service-worker-bundled.js`

### Key Configuration Files
- Service worker source: `/packages/host-web/src/service-worker.ts`
- Server implementation: `/examples/latergram/server/src/index.ts`
- Bundle adapter: `/examples/latergram/server/src/bundleStorageAdapter.ts`

## Architecture Benefits

1. **Dynamic Loading**: Applications can be loaded without rebuilding the runtime
2. **Virtual File System**: Provides file-like interface with real-time sync
3. **WASM Performance**: Core operations run at near-native speed
4. **Development Experience**: Hot reloading and real-time collaboration
5. **Bundle Portability**: `.tonk` files are self-contained application packages

## Future Considerations

- **Production Deployment**: Cache headers need adjustment for production use
- **Bundle Optimization**: Compression and selective loading for large applications  
- **Security**: Sandboxing and permission model for loaded applications
- **Multi-App Support**: Loading multiple `.tonk` applications simultaneously
- **Offline Support**: Caching strategy for offline application usage

---

*This architecture enables rapid development and deployment of collaborative applications with real-time synchronization and WASM-powered performance.*
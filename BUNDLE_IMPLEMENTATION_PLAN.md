# Implementation Plan for Tonk Bundle System

## Overview

This plan outlines the implementation of a bundling system that packages Tonk apps into `.tonk`
files according to the `@tonk/spec` format, and enables the browser package to load and render these
bundled applications.

## Phase 1: Bundling Script Creation

**1. Create bundling script for my-world app**

- Location: `/scripts/bundle-my-world.ts`
- Purpose: Specialized script for bundling the my-world example
- Key features:
  - Automatically builds the app first (`npm run build`)
  - Bundles the `dist/` folder contents
  - Includes `tonk.config.json` from root
  - Sets proper MIME types for all file types
  - Creates entrypoints for `main` → `/index.html`
  - Outputs `my-world.tonk` file

**2. Build process integration**

- Ensure Vite build completes successfully
- Handle the large file warning (adjust workbox config if needed)
- Preserve the exact directory structure from dist

**3. Bundle structure**

```
my-world.tonk (ZIP)
├── manifest.json (auto-generated)
├── index.html
├── assets/
│   ├── index-*.js
│   ├── vendor-*.js
│   ├── automerge-*.js
│   ├── automerge_wasm_bg-*.wasm
│   └── index-*.css
├── registerSW.js
├── manifest.webmanifest
└── tonk.config.json
```

## Phase 2: Browser Modifications

**4. Update file dialog to accept .tonk files**

- Modify `main.js`:
  - Add `.tonk` to file filters in dialog
  - Detect .tonk extension for special handling

**5. Enhance load-in.js for app serving**

- Current behavior: Extracts files to KeepSync
- New additions:
  - Create a virtual file system map in memory
  - Track the main entrypoint (`/index.html`)
  - Prepare files for serving via local HTTP server

**6. Add app rendering capability**

- Options: a. **Iframe approach** (simpler):
  - Create iframe pointing to local server URL
  - Serve extracted files from Express server b. **Service Worker approach** (more complex):
  - Intercept requests for bundled app files
  - Serve from extracted bundle data

**7. Local server setup for extracted files**

- Extend existing Express server in `main.js`
- Add dynamic route for bundled apps (e.g., `/tonk-apps/:bundleName/*`)
- Serve files from memory/KeepSync with correct MIME types
- Handle asset paths correctly (resolve `/assets/*` requests)

## Phase 3: UI Enhancements

**8. Update index.html UI**

- Add section for loaded Tonk apps
- Show bundle metadata (name, description, file count)
- Add "Launch App" button
- Display app in iframe or new tab

## Technical Details

### File Serving Strategy

```javascript
// In main.js - Express route for serving bundled app
app.get('/tonk-app/*', (req, res) => {
  const filePath = req.path.replace('/tonk-app', '');
  const fileData = getExtractedFile(filePath); // From KeepSync or memory
  res.contentType(fileData.mimeType);
  res.send(fileData.content);
});
```

### Bundle Loading Flow

1. User opens .tonk file via dialog
2. Browser receives file as Uint8Array
3. `parseBundle()` extracts manifest and file list
4. Files are extracted and stored (KeepSync or memory)
5. UI shows bundle info and "Launch" button
6. Launch creates iframe with src=`/tonk-app/index.html`
7. Express server handles requests for app files
8. App renders in iframe with full functionality

### Key Considerations

- **Asset Path Resolution**: Ensure relative paths in HTML/JS work correctly
- **CORS/Security**: Configure proper headers for local serving
- **WebAssembly**: Special handling for .wasm files (proper MIME type)
- **Service Workers**: May need to disable or handle specially
- **Hot Reload**: Consider if dev mode needs special handling

## File Structure After Implementation

```
/scripts/
  bundle-my-world.ts       # Bundling script

/examples/my-world/
  dist/                    # Built app (source for bundle)
  my-world.tonk           # Generated bundle file

/packages/browser/
  src/
    load-in.js            # Enhanced to handle app extraction
    index.html            # UI for displaying loaded apps
    app-renderer.js       # New: Handle app rendering
  main.js                 # Express server with tonk-app routes
```

## Success Criteria

1. ✅ Can build my-world app
2. ✅ Can create .tonk bundle with all dist files
3. ✅ Browser can open .tonk file
4. ✅ Bundle is correctly parsed and extracted
5. ✅ App files are served with correct MIME types
6. ✅ my-world app renders and functions in browser
7. ✅ All assets (JS, CSS, WASM) load correctly
8. ✅ App maintains full functionality (maps, data, etc.)

## Implementation Notes

- Start with Phase 1 to establish the bundling foundation
- Use my-world as the primary test case
- Focus on the iframe approach for initial implementation
- Ensure bundle format exactly matches @tonk/spec requirements
- Test with actual built assets, not source files

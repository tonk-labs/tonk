# Desktonk Desktop Environment - Design Document

**Date:** 2025-11-07 **Status:** Design Phase **Author:** Collaborative design session

## Overview

Desktonk transforms the Berlin text editor into a desktop environment. The text editor becomes one
launchable app among many. TLDraw renders files from the VFS `/desktonk/` directory as interactive
icons on an infinite canvas.

## Goals

- Create a browser-based desktop environment
- Display VFS files as draggable icons on a canvas
- Launch apps (starting with text editor) via double-click
- Support future expansion to Electron/Tauri native windows
- Maintain clean separation between desktop and apps

## Non-Goals (YAGNI)

- Window management within desktop (apps use routes, not floating windows)
- Folders/directory navigation on desktop (flat file list initially)
- Drag-drop file operations between apps
- Multi-user desktop collaboration (future)

## Architecture

### Routes

```
/                           → Desktop (TLDraw canvas)
/text-editor?file=...       → Text editor app (existing Berlin editor)
/image-viewer?file=...      → Future: image viewer
/settings                   → Future: desktop settings
```

Apps run separately from the desktop. Launching an app navigates to its route. This enables:

- SPA route navigation
- New browser tab opening
- Iframe embedding (if needed)
- Electron/Tauri window launching (future)

### VFS File Structure

```
/desktonk/
  my-document.txt      # Text file with content + optional desktopMeta
  photo.png            # Binary file with bytes field
  todo-list.json       # JSON data file
```

### File Metadata Pattern

Each file's `content` field contains the actual data plus optional `desktopMeta`:

```json
{
  "data": "actual file content here...",
  "desktopMeta": {
    "x": 100, // Position on desktop (optional, auto-placed if missing)
    "y": 200,
    "icon": "base64-or-url", // Custom icon (optional, falls back to MIME)
    "appHandler": "text-editor" // Which app opens it (optional, inferred from MIME)
  }
}
```

Binary files (images) store base64 content in the `bytes` field; `content` holds only metadata.

**MIME Detection:** The `mime` package detects type from file extension when no explicit metadata
exists.

## TLDraw Integration

### Custom FileIcon Shape

Define a custom TLDraw shape type representing desktop file icons:

```typescript
type FileIconShape = {
  type: 'file-icon';
  x: number;
  y: number;
  props: {
    filePath: string; // e.g., "/desktonk/my-doc.txt"
    fileName: string; // Display name
    mimeType: string; // For icon selection
    customIcon?: string; // Override icon if set in desktopMeta
    appHandler?: string; // Which app opens this
  };
};
```

### Shape Rendering

A custom React component renders each icon:

- Icon image (MIME-based or custom)
- File name label
- Visual states: normal, hover, selected, dragging
- Drag animations (scale, opacity, shadow)

TLDraw's custom shape API provides:

- State-based styling (`isDragging`, `isSelected`, `isHovered`)
- Animation hooks
- React component control
- Drag/drop lifecycle

### VFS ↔ TLDraw Sync

**Initialization:**

1. Desktop mounts, reads `/desktonk/` directory
2. Creates FileIcon shape for each file, reading position from `desktopMeta`
3. Auto-layouts files without positions

**Ongoing Sync:**

- **User drags icon** → TLDraw updates position → on drop, writes x,y to `desktopMeta` (debounced)
- **VFS adds file** → Creates FileIcon shape
- **VFS deletes file** → Removes shape
- **VFS changes file** → Updates shape props

**VFS is source of truth.** On reconnect or desync, rebuild shapes from VFS.

### Interactions

- **Single click**: Select icon (TLDraw built-in selection)
- **Double click**: Navigate to `/{appHandler}?file={filePath}`
- **Drag**: Move icon, sync position to VFS on drop (debounced)
- **Right click**: Context menu (future: rename, delete, properties)

## Component Structure

```
src/features/desktop/
  components/
    Desktop.tsx              # Main TLDraw canvas wrapper (full-screen)
    FileIconShape.tsx        # Custom shape definition + renderer
    FileIconUtil.ts          # Shape util (TLDraw lifecycle)
  hooks/
    useDesktopSync.ts        # VFS ↔ TLDraw shape sync
    useFileOperations.ts     # Create/delete/rename files
  stores/
    desktopStore.ts          # Desktop state (if needed)
  utils/
    iconResolver.ts          # MIME → icon mapping
    autoLayout.ts            # Auto-position new files
  types.ts
  index.ts

src/features/text-editor/
  components/
    Layout.tsx               # ← Moved from src/components (app-specific)
    Header.tsx               # ← Moved from src/components
    EditableTitle.tsx        # ← Moved from src/components
  TextEditorApp.tsx          # Route component wrapper
  Editor.tsx                 # Existing editor
  ...
```

### Layout Separation

**Desktop (Full-screen):**

```typescript
function Desktop() {
  return (
    <div style={{ width: '100vw', height: '100vh' }}>
      <Tldraw
        components={customComponents}
        shapeUtils={[FileIconUtil]}
        onDoubleClick={handleFileOpen}
      />
    </div>
  )
}
```

Pure TLDraw canvas fills the viewport—no header, no wrapper.

**Text Editor App (With Layout):**

```typescript
function TextEditorApp() {
  const params = new URLSearchParams(location.search)
  const filePath = params.get('file')

  return (
    <Layout>
      <Editor filePath={filePath} />
    </Layout>
  )
}
```

Layout, Header, and EditableTitle move into `src/features/text-editor/` as app-specific components.

## Data Flow

### Desktop Initialization

```
1. Mount Desktop component
2. useDesktopSync reads /desktonk/ directory
3. For each file:
   - Read desktopMeta for position (or auto-layout)
   - Detect MIME type from extension
   - Create FileIconShape
4. Set up VFS directory watcher
5. Render TLDraw with shapes
```

### App Launching

```
User double-clicks FileIcon
→ Determines app from desktopMeta.appHandler || MIME_TO_APP[mimeType]
→ Navigates to `/${appHandler}?file=${encodeURIComponent(filePath)}`
→ App reads file param
→ App loads file from VFS
→ App renders content
→ Back button returns to /
```

### File Creation

```typescript
const createFile = async (name: string, content: any) => {
  const newFile = {
    data: content,
    desktopMeta: {
      x: autoLayout.getNextPosition(),
      y: autoLayout.getNextPosition(),
    },
  };
  await vfs.writeFile(`/desktonk/${name}`, newFile, true);
  // VFS directory watcher triggers
  // → useDesktopSync creates new FileIcon shape
};
```

### Position Persistence

```
User drags FileIcon
→ TLDraw fires drag events
→ On drag end:
  - Debounce position update (avoid writes on every pixel)
  - Read current file from VFS
  - Update desktopMeta.x and desktopMeta.y
  - Write back to VFS
```

## Error Handling

**File not found:**

- Show error message in app
- Provide "Return to Desktop" button

**VFS disconnected:**

- Overlay: "Reconnecting..."
- Disable interactions
- Rebuild shapes on reconnect

**Invalid file metadata:**

- Missing position → auto-layout
- Missing icon → MIME-based icon
- Invalid data → empty file

**Shape/VFS desync:**

- VFS is source of truth
- Discard shapes and rebuild from VFS
- Ignore stale events during rebuild

**Race conditions:**

- Debounce position updates (500ms)
- Queue VFS writes, ignore duplicates
- Update shape only if position changed

## MIME to App Mapping

Default handler association:

```typescript
const MIME_TO_APP: Record<string, string> = {
  'text/plain': 'text-editor',
  'text/markdown': 'text-editor',
  'application/json': 'text-editor',
  'text/html': 'text-editor',
  // Future:
  // 'image/png': 'image-viewer',
  // 'image/jpeg': 'image-viewer',
  // 'application/pdf': 'pdf-viewer',
};
```

Files can override with `desktopMeta.appHandler`.

## Implementation Phases

### Phase 1: Desktop Foundation

- Set up routing (`/` for desktop, `/text-editor` for app)
- Install & configure TLDraw (`@tldraw/tldraw`)
- Create basic Desktop component with full-screen TLDraw
- VFS directory read of `/desktonk/`
- Basic file listing (verify with console.log)

### Phase 2: Custom FileIcon Shape

- Define FileIconShape type & props
- Create FileIconUtil (TLDraw shape lifecycle methods)
- Implement FileIconShape React component
- Render simple rectangles with file names (no icons yet)
- Verify shapes appear on canvas and are draggable

### Phase 3: VFS Sync

- Build useDesktopSync hook: VFS → TLDraw shapes
- Create shapes from `/desktonk/` files
- Read desktopMeta for positions, auto-layout if missing
- Directory watcher: add/remove shapes on file changes
- Position persistence: drag shape → debounced VFS write

### Phase 4: Icons & Polish

- MIME type detection via `mime` package
- Icon resolver (MIME → icon image mapping)
- Style states: hover, selected, dragging animations
- Auto-layout algorithm for new files (grid or smart placement)
- Support custom icons from desktopMeta

### Phase 5: App Launching

- Double-click detection on FileIcon shapes
- App handler resolution (desktopMeta.appHandler || MIME_TO_APP)
- Navigate to `/text-editor?file=...`
- Refactor text editor:
  - Move to `src/features/text-editor/`
  - Move Layout/Header components into text-editor feature
  - Create TextEditorApp route wrapper
  - Read file param, load from VFS
- Back navigation to desktop

## Open Questions

1. Auto-layout algorithm: grid-based or smart freeform placement?
2. Initial desktop state: empty, or create sample "Welcome.txt"?
3. Context menu: native browser menu or custom React component?
4. File name editing: inline edit on desktop or in properties dialog?
5. TLDraw persistence: sync TLDraw document to VFS or rebuild on mount?

## Future Enhancements

- Multiple desktops/workspaces
- Folder support (directories on desktop)
- Grid snap and alignment guides
- Desktop background customization
- Keyboard shortcuts (Cmd+O to open, Delete to remove)
- Drag-drop files from OS into desktop
- App windows (overlay mode instead of routes)
- Electron/Tauri native window launching

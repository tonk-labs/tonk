# Desktonk Desktop Environment Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan
> task-by-task.

**Goal:** Transform Berlin text editor into a desktop environment using TLDraw, where files from VFS
`/desktonk/` appear as draggable icons and launch apps on double-click.

**Architecture:** Desktop route (/) renders TLDraw canvas with custom FileIcon shapes. Text editor
moves to `/text-editor?file=...` route. VFS directory watcher syncs file changes to TLDraw shapes.
MIME types determine default app handlers.

**Tech Stack:** React 19, TLDraw, React Router, Zustand, Tonk VFS, TypeScript, mime package

---

## Task 1: Install Dependencies and Setup Routing

**Files:**

- Modify: `app/package.json`
- Create: `app/src/Router.tsx`
- Modify: `app/src/main.tsx`

**Step 1: Install TLDraw and React Router**

```bash
cd app
bun add tldraw react-router-dom
bun add -D @types/react-router-dom
```

Expected: Dependencies added to package.json

**Step 2: Create Router component**

Create `app/src/Router.tsx`:

```typescript
import { BrowserRouter, Routes, Route } from 'react-router-dom';
import App from './App';

function Router() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<div>Desktop placeholder</div>} />
        <Route path="/text-editor" element={<App />} />
      </Routes>
    </BrowserRouter>
  );
}

export default Router;
```

**Step 3: Update main.tsx to use Router**

Replace `app/src/main.tsx` content:

```typescript
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import Router from "./Router";
import { getVFSService } from "./lib/vfs-service";

// Initialize VFS before React mounts
getVFSService().initialize('', '').catch(err => {
  console.warn('VFS initialization warning:', err);
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Router />
  </StrictMode>,
);
```

**Step 4: Verify routing works**

Run: `bun run dev`

Navigate to:

- `http://localhost:5173/` → Should show "Desktop placeholder"
- `http://localhost:5173/text-editor` → Should show text editor with chat

**Step 5: Commit**

```bash
git add app/package.json app/src/Router.tsx app/src/main.tsx
git commit -m "feat(desktonk): add routing infrastructure for desktop and apps"
```

---

## Task 2: Create Desktop Feature Structure

**Files:**

- Create: `app/src/features/desktop/index.ts`
- Create: `app/src/features/desktop/types.ts`
- Create: `app/src/features/desktop/components/Desktop.tsx`

**Step 1: Create types file**

Create `app/src/features/desktop/types.ts`:

```typescript
export interface DesktopFile {
  path: string;
  name: string;
  mimeType: string;
  desktopMeta?: {
    x?: number;
    y?: number;
    icon?: string;
    appHandler?: string;
  };
}

export interface FileIconShapeProps {
  filePath: string;
  fileName: string;
  mimeType: string;
  customIcon?: string;
  appHandler?: string;
}
```

**Step 2: Create Desktop component stub**

Create `app/src/features/desktop/components/Desktop.tsx`:

```typescript
function Desktop() {
  return (
    <div style={{ width: '100vw', height: '100vh', background: '#1a1a1a' }}>
      <div style={{ color: 'white', padding: '20px' }}>
        Desktop - TLDraw will go here
      </div>
    </div>
  );
}

export default Desktop;
```

**Step 3: Create feature index**

Create `app/src/features/desktop/index.ts`:

```typescript
export { default as Desktop } from './components/Desktop';
export type { DesktopFile, FileIconShapeProps } from './types';
```

**Step 4: Update Router to use Desktop**

Modify `app/src/Router.tsx`:

```typescript
import { BrowserRouter, Routes, Route } from 'react-router-dom';
import { Desktop } from './features/desktop';
import App from './App';

function Router() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Desktop />} />
        <Route path="/text-editor" element={<App />} />
      </Routes>
    </BrowserRouter>
  );
}

export default Router;
```

**Step 5: Verify Desktop renders**

Run: `bun run dev` Navigate to `http://localhost:5173/` Expected: Black screen with "Desktop -
TLDraw will go here"

**Step 6: Commit**

```bash
git add app/src/features/desktop/ app/src/Router.tsx
git commit -m "feat(desktop): create desktop feature structure"
```

---

## Task 3: Integrate TLDraw Canvas

**Files:**

- Modify: `app/src/features/desktop/components/Desktop.tsx`
- Create: `app/src/features/desktop/components/desktop.css`

**Step 1: Create CSS file**

Create `app/src/features/desktop/components/desktop.css`:

```css
.desktop-container {
  position: fixed;
  inset: 0;
  width: 100vw;
  height: 100vh;
}

.tldraw-container {
  width: 100%;
  height: 100%;
}
```

**Step 2: Add TLDraw to Desktop component**

Replace `app/src/features/desktop/components/Desktop.tsx`:

```typescript
import { Tldraw } from 'tldraw';
import 'tldraw/tldraw.css';
import './desktop.css';

function Desktop() {
  return (
    <div className="desktop-container">
      <Tldraw
        className="tldraw-container"
      />
    </div>
  );
}

export default Desktop;
```

**Step 3: Verify TLDraw renders**

Run: `bun run dev` Navigate to `http://localhost:5173/` Expected: TLDraw canvas with drawing tools
visible

**Step 4: Commit**

```bash
git add app/src/features/desktop/components/
git commit -m "feat(desktop): integrate TLDraw canvas"
```

---

## Task 4: Create FileIcon Custom Shape Definition

**Files:**

- Create: `app/src/features/desktop/shapes/FileIconShape.ts`
- Create: `app/src/features/desktop/shapes/FileIconUtil.tsx`
- Create: `app/src/features/desktop/shapes/types.ts`

**Step 1: Create shape types**

Create `app/src/features/desktop/shapes/types.ts`:

```typescript
import { TLBaseShape, RecordProps } from 'tldraw';

export type FileIconShape = TLBaseShape<
  'file-icon',
  {
    filePath: string;
    fileName: string;
    mimeType: string;
    customIcon?: string;
    appHandler?: string;
    w: number;
    h: number;
  }
>;
```

**Step 2: Create shape definition**

Create `app/src/features/desktop/shapes/FileIconShape.ts`:

```typescript
import { defineShape } from 'tldraw';
import type { FileIconShape } from './types';

export const fileIconShape = defineShape<FileIconShape>({
  type: 'file-icon',
  getDefaultProps(): FileIconShape['props'] {
    return {
      filePath: '',
      fileName: 'Untitled',
      mimeType: 'application/octet-stream',
      w: 80,
      h: 100,
    };
  },
});
```

**Step 3: Create shape util**

Create `app/src/features/desktop/shapes/FileIconUtil.tsx`:

```typescript
import { HTMLContainer, ShapeUtil, TLOnResizeHandler } from 'tldraw';
import type { FileIconShape } from './types';

export class FileIconUtil extends ShapeUtil<FileIconShape> {
  static override type = 'file-icon' as const;

  override isAspectRatioLocked = () => false;
  override canResize = () => true;
  override canBind = () => false;

  getDefaultProps(): FileIconShape['props'] {
    return {
      filePath: '',
      fileName: 'Untitled',
      mimeType: 'application/octet-stream',
      w: 80,
      h: 100,
    };
  }

  getGeometry(shape: FileIconShape) {
    return {
      type: 'rectangle' as const,
      w: shape.props.w,
      h: shape.props.h,
    };
  }

  component(shape: FileIconShape) {
    return (
      <HTMLContainer
        style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          pointerEvents: 'all',
          backgroundColor: '#2d2d2d',
          border: '2px solid #444',
          borderRadius: '8px',
          padding: '8px',
        }}
      >
        <div
          style={{
            fontSize: '32px',
            marginBottom: '8px',
          }}
        >
          📄
        </div>
        <div
          style={{
            fontSize: '12px',
            color: '#fff',
            textAlign: 'center',
            wordBreak: 'break-word',
            maxWidth: '100%',
          }}
        >
          {shape.props.fileName}
        </div>
      </HTMLContainer>
    );
  }

  indicator(shape: FileIconShape) {
    return (
      <rect
        width={shape.props.w}
        height={shape.props.h}
        fill="transparent"
        stroke="blue"
        strokeWidth={2}
      />
    );
  }

  override onResize: TLOnResizeHandler<FileIconShape> = (shape, info) => {
    return {
      props: {
        w: Math.max(60, info.newSize.x),
        h: Math.max(80, info.newSize.y),
      },
    };
  };
}
```

**Step 4: Verify shape definition compiles**

Run: `bun run build` Expected: No TypeScript errors

**Step 5: Commit**

```bash
git add app/src/features/desktop/shapes/
git commit -m "feat(desktop): create FileIcon custom shape definition"
```

---

## Task 5: Register Custom Shape with TLDraw

**Files:**

- Modify: `app/src/features/desktop/components/Desktop.tsx`
- Create: `app/src/features/desktop/shapes/index.ts`

**Step 1: Create shapes index**

Create `app/src/features/desktop/shapes/index.ts`:

```typescript
export { FileIconUtil } from './FileIconUtil';
export type { FileIconShape } from './types';
```

**Step 2: Register shape with TLDraw**

Modify `app/src/features/desktop/components/Desktop.tsx`:

```typescript
import { Tldraw } from 'tldraw';
import 'tldraw/tldraw.css';
import './desktop.css';
import { FileIconUtil } from '../shapes';

const customShapeUtils = [FileIconUtil];

function Desktop() {
  return (
    <div className="desktop-container">
      <Tldraw
        className="tldraw-container"
        shapeUtils={customShapeUtils}
      />
    </div>
  );
}

export default Desktop;
```

**Step 3: Verify custom shape is available**

Run: `bun run dev` Navigate to `http://localhost:5173/` Open browser console and type:

```javascript
editor.createShape({
  type: 'file-icon',
  props: { fileName: 'test.txt', filePath: '/test.txt', mimeType: 'text/plain' },
});
```

Expected: File icon shape appears on canvas with document emoji and "test.txt" label

**Step 4: Commit**

```bash
git add app/src/features/desktop/
git commit -m "feat(desktop): register FileIcon shape with TLDraw"
```

---

## Task 6: Create VFS Sync Hook

**Files:**

- Create: `app/src/features/desktop/hooks/useDesktopSync.ts`
- Create: `app/src/features/desktop/utils/fileMetadata.ts`

**Step 1: Create file metadata utilities**

Create `app/src/features/desktop/utils/fileMetadata.ts`:

```typescript
import type { DocumentData } from '@tonk/core';
import type { DesktopFile } from '../types';

export function extractDesktopFile(path: string, doc: DocumentData): DesktopFile {
  const content = doc.content as any;
  const desktopMeta = content?.desktopMeta;

  return {
    path,
    name: doc.name,
    mimeType: desktopMeta?.mimeType || 'application/octet-stream',
    desktopMeta: {
      x: desktopMeta?.x,
      y: desktopMeta?.y,
      icon: desktopMeta?.icon,
      appHandler: desktopMeta?.appHandler,
    },
  };
}

export function getNextAutoLayoutPosition(index: number): { x: number; y: number } {
  const gridSize = 120;
  const columns = 8;

  const col = index % columns;
  const row = Math.floor(index / columns);

  return {
    x: 50 + col * gridSize,
    y: 50 + row * gridSize,
  };
}
```

**Step 2: Create useDesktopSync hook**

Create `app/src/features/desktop/hooks/useDesktopSync.ts`:

```typescript
import { useEffect, useState } from 'react';
import { useEditor } from 'tldraw';
import { getVFSService } from '../../../lib/vfs-service';
import { extractDesktopFile, getNextAutoLayoutPosition } from '../utils/fileMetadata';
import type { DesktopFile } from '../types';

export function useDesktopSync() {
  const editor = useEditor();
  const [files, setFiles] = useState<DesktopFile[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    const vfs = getVFSService();
    let watchId: string | null = null;

    async function loadDesktopFiles() {
      try {
        const entries = await vfs.listDirectory('/desktonk');
        const filePromises = entries
          .filter(entry => entry.type === 'document')
          .map(async entry => {
            const doc = await vfs.readFile(`/desktonk/${entry.name}`);
            return extractDesktopFile(`/desktonk/${entry.name}`, doc);
          });

        const desktopFiles = await Promise.all(filePromises);
        setFiles(desktopFiles);

        // Create TLDraw shapes for each file
        desktopFiles.forEach((file, index) => {
          const position =
            file.desktopMeta?.x && file.desktopMeta?.y
              ? { x: file.desktopMeta.x, y: file.desktopMeta.y }
              : getNextAutoLayoutPosition(index);

          editor.createShape({
            type: 'file-icon',
            x: position.x,
            y: position.y,
            props: {
              filePath: file.path,
              fileName: file.name,
              mimeType: file.mimeType,
              customIcon: file.desktopMeta?.icon,
              appHandler: file.desktopMeta?.appHandler,
              w: 80,
              h: 100,
            },
          });
        });

        setIsLoading(false);
      } catch (error) {
        console.error('Failed to load desktop files:', error);
        setIsLoading(false);
      }
    }

    // Setup directory watcher
    async function setupWatcher() {
      try {
        watchId = await vfs.watchDirectory('/desktonk', changeData => {
          console.log('Directory changed:', changeData);
          // Reload files on change
          loadDesktopFiles();
        });
      } catch (error) {
        console.error('Failed to setup directory watcher:', error);
      }
    }

    if (vfs.isInitialized()) {
      loadDesktopFiles();
      setupWatcher();
    }

    return () => {
      if (watchId) {
        vfs.unwatchDirectory(watchId).catch(console.error);
      }
    };
  }, [editor]);

  return { files, isLoading };
}
```

**Step 3: Create utils index**

Create `app/src/features/desktop/utils/index.ts`:

```typescript
export * from './fileMetadata';
```

**Step 4: Create hooks index**

Create `app/src/features/desktop/hooks/index.ts`:

```typescript
export { useDesktopSync } from './useDesktopSync';
```

**Step 5: Verify hook compiles**

Run: `bun run build` Expected: No TypeScript errors

**Step 6: Commit**

```bash
git add app/src/features/desktop/hooks/ app/src/features/desktop/utils/
git commit -m "feat(desktop): create VFS sync hook"
```

---

## Task 7: Integrate VFS Sync with Desktop

**Files:**

- Modify: `app/src/features/desktop/components/Desktop.tsx`

**Step 1: Add useDesktopSync to Desktop**

Modify `app/src/features/desktop/components/Desktop.tsx`:

```typescript
import { Tldraw, track } from 'tldraw';
import 'tldraw/tldraw.css';
import './desktop.css';
import { FileIconUtil } from '../shapes';
import { useDesktopSync } from '../hooks';

const customShapeUtils = [FileIconUtil];

const DesktopInner = track(() => {
  const { files, isLoading } = useDesktopSync();

  if (isLoading) {
    return (
      <div className="desktop-container" style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        background: '#1a1a1a',
        color: '#fff'
      }}>
        Loading desktop...
      </div>
    );
  }

  return null;
});

function Desktop() {
  return (
    <div className="desktop-container">
      <Tldraw
        className="tldraw-container"
        shapeUtils={customShapeUtils}
      >
        <DesktopInner />
      </Tldraw>
    </div>
  );
}

export default Desktop;
```

**Step 2: Create test file in VFS**

Open browser console at `http://localhost:5173/`:

```javascript
// Get VFS
const vfs = window.getVFSService();

// Create test directory if needed
await vfs.writeFile(
  '/desktonk/test.txt',
  {
    content: {
      data: 'Hello from desktop!',
      desktopMeta: {
        x: 100,
        y: 100,
      },
    },
  },
  true
);
```

**Step 3: Verify file appears as icon**

Refresh page at `http://localhost:5173/` Expected: File icon labeled "test.txt" appears at position
(100, 100)

**Step 4: Commit**

```bash
git add app/src/features/desktop/components/Desktop.tsx
git commit -m "feat(desktop): integrate VFS sync with desktop canvas"
```

---

## Task 8: Add MIME Type Detection

**Files:**

- Create: `app/src/features/desktop/utils/mimeResolver.ts`
- Modify: `app/src/features/desktop/utils/fileMetadata.ts`

**Step 1: Create MIME resolver**

Create `app/src/features/desktop/utils/mimeResolver.ts`:

```typescript
import mime from 'mime';

export function getMimeType(fileName: string): string {
  const detected = mime.getType(fileName);
  return detected || 'application/octet-stream';
}

export function getFileIcon(mimeType: string): string {
  if (mimeType.startsWith('text/')) return '📄';
  if (mimeType.startsWith('image/')) return '🖼️';
  if (mimeType === 'application/json') return '📋';
  if (mimeType === 'application/pdf') return '📕';
  if (mimeType.startsWith('video/')) return '🎬';
  if (mimeType.startsWith('audio/')) return '🎵';
  return '📦';
}

export const MIME_TO_APP: Record<string, string> = {
  'text/plain': 'text-editor',
  'text/markdown': 'text-editor',
  'application/json': 'text-editor',
  'text/html': 'text-editor',
  // Future apps can be added here
};

export function getAppHandler(mimeType: string, override?: string): string {
  return override || MIME_TO_APP[mimeType] || 'text-editor';
}
```

**Step 2: Update fileMetadata to use MIME detection**

Modify `app/src/features/desktop/utils/fileMetadata.ts`:

```typescript
import type { DocumentData } from '@tonk/core';
import type { DesktopFile } from '../types';
import { getMimeType } from './mimeResolver';

export function extractDesktopFile(path: string, doc: DocumentData): DesktopFile {
  const content = doc.content as any;
  const desktopMeta = content?.desktopMeta;

  // Detect MIME type from filename if not in metadata
  const detectedMime = getMimeType(doc.name);
  const mimeType = desktopMeta?.mimeType || detectedMime;

  return {
    path,
    name: doc.name,
    mimeType,
    desktopMeta: {
      x: desktopMeta?.x,
      y: desktopMeta?.y,
      icon: desktopMeta?.icon,
      appHandler: desktopMeta?.appHandler,
    },
  };
}

export function getNextAutoLayoutPosition(index: number): { x: number; y: number } {
  const gridSize = 120;
  const columns = 8;

  const col = index % columns;
  const row = Math.floor(index / columns);

  return {
    x: 50 + col * gridSize,
    y: 50 + row * gridSize,
  };
}
```

**Step 3: Update FileIconUtil to show MIME-based icon**

Modify `app/src/features/desktop/shapes/FileIconUtil.tsx`:

```typescript
import { HTMLContainer, ShapeUtil, TLOnResizeHandler } from 'tldraw';
import type { FileIconShape } from './types';
import { getFileIcon } from '../utils/mimeResolver';

export class FileIconUtil extends ShapeUtil<FileIconShape> {
  static override type = 'file-icon' as const;

  override isAspectRatioLocked = () => false;
  override canResize = () => true;
  override canBind = () => false;

  getDefaultProps(): FileIconShape['props'] {
    return {
      filePath: '',
      fileName: 'Untitled',
      mimeType: 'application/octet-stream',
      w: 80,
      h: 100,
    };
  }

  getGeometry(shape: FileIconShape) {
    return {
      type: 'rectangle' as const,
      w: shape.props.w,
      h: shape.props.h,
    };
  }

  component(shape: FileIconShape) {
    const icon = shape.props.customIcon || getFileIcon(shape.props.mimeType);

    return (
      <HTMLContainer
        style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          pointerEvents: 'all',
          backgroundColor: '#2d2d2d',
          border: '2px solid #444',
          borderRadius: '8px',
          padding: '8px',
        }}
      >
        <div
          style={{
            fontSize: '32px',
            marginBottom: '8px',
          }}
        >
          {icon}
        </div>
        <div
          style={{
            fontSize: '12px',
            color: '#fff',
            textAlign: 'center',
            wordBreak: 'break-word',
            maxWidth: '100%',
          }}
        >
          {shape.props.fileName}
        </div>
      </HTMLContainer>
    );
  }

  indicator(shape: FileIconShape) {
    return (
      <rect
        width={shape.props.w}
        height={shape.props.h}
        fill="transparent"
        stroke="blue"
        strokeWidth={2}
      />
    );
  }

  override onResize: TLOnResizeHandler<FileIconShape> = (shape, info) => {
    return {
      props: {
        w: Math.max(60, info.newSize.x),
        h: Math.max(80, info.newSize.y),
      },
    };
  };
}
```

**Step 4: Update utils index**

Modify `app/src/features/desktop/utils/index.ts`:

```typescript
export * from './fileMetadata';
export * from './mimeResolver';
```

**Step 5: Test different file types**

Create test files via console:

```javascript
const vfs = window.getVFSService();

await vfs.writeFile('/desktonk/document.txt', { content: { data: 'text' } }, true);
await vfs.writeFile('/desktonk/data.json', { content: { data: {} } }, true);
await vfs.writeFile('/desktonk/readme.md', { content: { data: '# Title' } }, true);
```

Expected: Different icons for each file type (📄 for .txt, 📋 for .json, etc.)

**Step 6: Commit**

```bash
git add app/src/features/desktop/utils/ app/src/features/desktop/shapes/FileIconUtil.tsx
git commit -m "feat(desktop): add MIME type detection and icons"
```

---

## Task 9: Implement Double-Click App Launching

**Files:**

- Modify: `app/src/features/desktop/components/Desktop.tsx`
- Modify: `app/src/features/desktop/shapes/FileIconUtil.tsx`

**Step 1: Add double-click handler to Desktop**

Modify `app/src/features/desktop/components/Desktop.tsx`:

```typescript
import { Tldraw, track, useEditor } from 'tldraw';
import { useNavigate } from 'react-router-dom';
import 'tldraw/tldraw.css';
import './desktop.css';
import { FileIconUtil } from '../shapes';
import { useDesktopSync } from '../hooks';
import type { FileIconShape } from '../shapes';
import { getAppHandler } from '../utils/mimeResolver';

const customShapeUtils = [FileIconUtil];

const DesktopInner = track(() => {
  const editor = useEditor();
  const navigate = useNavigate();
  const { files, isLoading } = useDesktopSync();

  // Handle double-click on shapes
  const handleDoubleClick = (shape: FileIconShape) => {
    const { filePath, mimeType, appHandler } = shape.props;
    const targetApp = getAppHandler(mimeType, appHandler);
    const encodedPath = encodeURIComponent(filePath);
    navigate(`/${targetApp}?file=${encodedPath}`);
  };

  // Listen for double-click events
  editor.on('double-click-shape', (data) => {
    const shape = editor.getShape(data.shapeId);
    if (shape && shape.type === 'file-icon') {
      handleDoubleClick(shape as FileIconShape);
    }
  });

  if (isLoading) {
    return (
      <div className="desktop-container" style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        background: '#1a1a1a',
        color: '#fff'
      }}>
        Loading desktop...
      </div>
    );
  }

  return null;
});

function Desktop() {
  return (
    <div className="desktop-container">
      <Tldraw
        className="tldraw-container"
        shapeUtils={customShapeUtils}
      >
        <DesktopInner />
      </Tldraw>
    </div>
  );
}

export default Desktop;
```

**Step 2: Test double-click launching**

Navigate to `http://localhost:5173/` Double-click on a file icon Expected: Navigate to
`/text-editor?file=%2Fdesktonk%2Ftest.txt`

**Step 3: Commit**

```bash
git add app/src/features/desktop/components/Desktop.tsx
git commit -m "feat(desktop): implement double-click app launching"
```

---

## Task 10: Move Text Editor to Dedicated Route Component

**Files:**

- Create: `app/src/features/text-editor/TextEditorApp.tsx`
- Create: `app/src/features/text-editor/index.ts`
- Modify: `app/src/Router.tsx`

**Step 1: Create TextEditorApp wrapper**

Create `app/src/features/text-editor/TextEditorApp.tsx`:

```typescript
import { useEffect } from 'react';
import { useSearchParams, useNavigate } from 'react-router-dom';
import Layout from '../../components/layout/layout';
import { Editor } from '../editor';
import { usePresenceTracking } from '../presence';
import { ChatWindow, useChat } from '../chat';
import { Button } from '../../components/ui/button/button';
import { getVFSService } from '../../lib/vfs-service';

function TextEditorApp() {
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const filePath = searchParams.get('file');

  // Enable presence tracking
  usePresenceTracking();

  // Chat functionality
  const { toggleWindow, windowState } = useChat();

  // Load file if path provided
  useEffect(() => {
    if (filePath) {
      const vfs = getVFSService();
      vfs.readFile(filePath)
        .then(doc => {
          console.log('Loaded file:', filePath, doc);
          // File loaded - editor will sync with VFS automatically
        })
        .catch(err => {
          console.error('Failed to load file:', err);
          // Show error and return to desktop
          alert(`File not found: ${filePath}`);
          navigate('/');
        });
    }
  }, [filePath, navigate]);

  if (!filePath) {
    return (
      <div style={{ padding: '20px', color: 'white' }}>
        <p>No file specified</p>
        <button onClick={() => navigate('/')}>Return to Desktop</button>
      </div>
    );
  }

  return (
    <>
      <Layout>
        <Editor/>
      </Layout>

      {/* Intercom-style floating chat button */}
      <Button
        variant="default"
        onClick={toggleWindow}
        className="fixed bottom-6 right-6 h-16 w-16 rounded-full shadow-lg hover:shadow-xl transition-all hover:scale-105 z-50 p-0"
        aria-label={windowState.isOpen ? "Close chat" : "Open chat"}
      >
        <svg
          width="32"
          height="32"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </svg>
      </Button>

      {/* Chat window */}
      <ChatWindow />
    </>
  );
}

export default TextEditorApp;
```

**Step 2: Create text-editor feature index**

Create `app/src/features/text-editor/index.ts`:

```typescript
export { default as TextEditorApp } from './TextEditorApp';
```

**Step 3: Update Router to use TextEditorApp**

Modify `app/src/Router.tsx`:

```typescript
import { BrowserRouter, Routes, Route } from 'react-router-dom';
import { Desktop } from './features/desktop';
import { TextEditorApp } from './features/text-editor';

function Router() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Desktop />} />
        <Route path="/text-editor" element={<TextEditorApp />} />
      </Routes>
    </BrowserRouter>
  );
}

export default Router;
```

**Step 4: Test file opening from desktop**

Navigate to `http://localhost:5173/` Double-click a file icon Expected: Navigate to text editor with
file loaded

**Step 5: Commit**

```bash
git add app/src/features/text-editor/ app/src/Router.tsx
git commit -m "feat(text-editor): create dedicated route component"
```

---

## Task 11: Persist Icon Positions to VFS

**Files:**

- Create: `app/src/features/desktop/hooks/usePositionSync.ts`
- Modify: `app/src/features/desktop/components/Desktop.tsx`

**Step 1: Create position sync hook**

Create `app/src/features/desktop/hooks/usePositionSync.ts`:

```typescript
import { useEffect, useRef } from 'react';
import { useEditor } from 'tldraw';
import type { FileIconShape } from '../shapes';
import { getVFSService } from '../../../lib/vfs-service';

export function usePositionSync() {
  const editor = useEditor();
  const saveTimeoutRef = useRef<Record<string, NodeJS.Timeout>>({});

  useEffect(() => {
    // Listen for shape changes
    const unsubscribe = editor.sideEffects.registerAfterChangeHandler('shape', (prev, next) => {
      if (next.type !== 'file-icon') return;

      const prevShape = prev as FileIconShape;
      const nextShape = next as FileIconShape;

      // Only save if position changed
      if (prevShape.x === nextShape.x && prevShape.y === nextShape.y) {
        return;
      }

      // Debounce save
      const shapeId = nextShape.id;
      if (saveTimeoutRef.current[shapeId]) {
        clearTimeout(saveTimeoutRef.current[shapeId]);
      }

      saveTimeoutRef.current[shapeId] = setTimeout(async () => {
        await savePosition(nextShape);
        delete saveTimeoutRef.current[shapeId];
      }, 500);
    });

    return () => {
      unsubscribe();
      // Clear pending saves
      Object.values(saveTimeoutRef.current).forEach(clearTimeout);
    };
  }, [editor]);
}

async function savePosition(shape: FileIconShape) {
  try {
    const vfs = getVFSService();
    const doc = await vfs.readFile(shape.props.filePath);
    const content = doc.content as any;

    const updatedContent = {
      ...content,
      desktopMeta: {
        ...content?.desktopMeta,
        x: shape.x,
        y: shape.y,
      },
    };

    await vfs.writeFile(shape.props.filePath, { content: updatedContent });
    console.log(`Saved position for ${shape.props.fileName}:`, { x: shape.x, y: shape.y });
  } catch (error) {
    console.error('Failed to save position:', error);
  }
}
```

**Step 2: Add position sync to Desktop**

Modify `app/src/features/desktop/components/Desktop.tsx`:

```typescript
import { Tldraw, track, useEditor } from 'tldraw';
import { useNavigate } from 'react-router-dom';
import 'tldraw/tldraw.css';
import './desktop.css';
import { FileIconUtil } from '../shapes';
import { useDesktopSync, usePositionSync } from '../hooks';
import type { FileIconShape } from '../shapes';
import { getAppHandler } from '../utils/mimeResolver';

const customShapeUtils = [FileIconUtil];

const DesktopInner = track(() => {
  const editor = useEditor();
  const navigate = useNavigate();
  const { files, isLoading } = useDesktopSync();

  // Enable position persistence
  usePositionSync();

  // Handle double-click on shapes
  const handleDoubleClick = (shape: FileIconShape) => {
    const { filePath, mimeType, appHandler } = shape.props;
    const targetApp = getAppHandler(mimeType, appHandler);
    const encodedPath = encodeURIComponent(filePath);
    navigate(`/${targetApp}?file=${encodedPath}`);
  };

  // Listen for double-click events
  editor.on('double-click-shape', (data) => {
    const shape = editor.getShape(data.shapeId);
    if (shape && shape.type === 'file-icon') {
      handleDoubleClick(shape as FileIconShape);
    }
  });

  if (isLoading) {
    return (
      <div className="desktop-container" style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        background: '#1a1a1a',
        color: '#fff'
      }}>
        Loading desktop...
      </div>
    );
  }

  return null;
});

function Desktop() {
  return (
    <div className="desktop-container">
      <Tldraw
        className="tldraw-container"
        shapeUtils={customShapeUtils}
      >
        <DesktopInner />
      </Tldraw>
    </div>
  );
}

export default Desktop;
```

**Step 3: Update hooks index**

Modify `app/src/features/desktop/hooks/index.ts`:

```typescript
export { useDesktopSync } from './useDesktopSync';
export { usePositionSync } from './usePositionSync';
```

**Step 4: Test position persistence**

Navigate to `http://localhost:5173/` Drag a file icon to new position Wait 1 second Refresh page
Expected: File icon appears at new position

**Step 5: Commit**

```bash
git add app/src/features/desktop/hooks/ app/src/features/desktop/components/Desktop.tsx
git commit -m "feat(desktop): persist icon positions to VFS"
```

---

## Task 12: Add Visual Polish to FileIcon

**Files:**

- Modify: `app/src/features/desktop/shapes/FileIconUtil.tsx`
- Create: `app/src/features/desktop/shapes/fileIcon.css`

**Step 1: Create CSS for file icon**

Create `app/src/features/desktop/shapes/fileIcon.css`:

```css
.file-icon {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  pointer-events: all;
  background: linear-gradient(135deg, #2d2d2d 0%, #1a1a1a 100%);
  border: 2px solid #444;
  border-radius: 12px;
  padding: 12px;
  transition: all 0.2s ease;
  user-select: none;
}

.file-icon:hover {
  background: linear-gradient(135deg, #3d3d3d 0%, #2a2a2a 100%);
  border-color: #666;
  transform: translateY(-2px);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
}

.file-icon.selected {
  border-color: #0078d4;
  background: linear-gradient(135deg, #2d3d4d 0%, #1a2a3a 100%);
}

.file-icon.dragging {
  opacity: 0.7;
  transform: scale(1.05);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.6);
}

.file-icon-emoji {
  font-size: 40px;
  margin-bottom: 8px;
  filter: drop-shadow(0 2px 4px rgba(0, 0, 0, 0.3));
}

.file-icon-label {
  font-size: 13px;
  color: #fff;
  text-align: center;
  word-break: break-word;
  max-width: 100%;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.5);
  font-weight: 500;
}
```

**Step 2: Update FileIconUtil with polished styles**

Modify `app/src/features/desktop/shapes/FileIconUtil.tsx`:

```typescript
import { HTMLContainer, ShapeUtil, TLOnResizeHandler } from 'tldraw';
import type { FileIconShape } from './types';
import { getFileIcon } from '../utils/mimeResolver';
import './fileIcon.css';

export class FileIconUtil extends ShapeUtil<FileIconShape> {
  static override type = 'file-icon' as const;

  override isAspectRatioLocked = () => false;
  override canResize = () => true;
  override canBind = () => false;

  getDefaultProps(): FileIconShape['props'] {
    return {
      filePath: '',
      fileName: 'Untitled',
      mimeType: 'application/octet-stream',
      w: 90,
      h: 110,
    };
  }

  getGeometry(shape: FileIconShape) {
    return {
      type: 'rectangle' as const,
      w: shape.props.w,
      h: shape.props.h,
    };
  }

  component(shape: FileIconShape) {
    const icon = shape.props.customIcon || getFileIcon(shape.props.mimeType);
    const isSelected = this.editor.getSelectedShapeIds().includes(shape.id);
    const isDragging = this.editor.getInstanceState().isDragging;

    const className = [
      'file-icon',
      isSelected ? 'selected' : '',
      isDragging ? 'dragging' : '',
    ].filter(Boolean).join(' ');

    return (
      <HTMLContainer className={className}>
        <div className="file-icon-emoji">{icon}</div>
        <div className="file-icon-label">{shape.props.fileName}</div>
      </HTMLContainer>
    );
  }

  indicator(shape: FileIconShape) {
    return (
      <rect
        width={shape.props.w}
        height={shape.props.h}
        fill="transparent"
        stroke="#0078d4"
        strokeWidth={3}
        rx={12}
      />
    );
  }

  override onResize: TLOnResizeHandler<FileIconShape> = (shape, info) => {
    return {
      props: {
        w: Math.max(70, info.newSize.x),
        h: Math.max(90, info.newSize.y),
      },
    };
  };
}
```

**Step 3: Verify polished appearance**

Run: `bun run dev` Navigate to `http://localhost:5173/` Expected: File icons have gradients, hover
effects, and smooth animations

**Step 4: Commit**

```bash
git add app/src/features/desktop/shapes/
git commit -m "feat(desktop): add visual polish to file icons"
```

---

## Task 13: Add Back Navigation from Text Editor

**Files:**

- Modify: `app/src/features/text-editor/TextEditorApp.tsx`
- Modify: `app/src/components/header/header.tsx`

**Step 1: Add back button to header**

Modify `app/src/components/header/header.tsx`:

```typescript
import { useNavigate } from 'react-router-dom';
import EditableTitle from './editable-title';

export default function Header() {
  const navigate = useNavigate();

  return (
    <header className="header">
      <button
        onClick={() => navigate('/')}
        className="back-button"
        aria-label="Back to Desktop"
        style={{
          background: 'transparent',
          border: 'none',
          color: '#fff',
          fontSize: '24px',
          cursor: 'pointer',
          padding: '8px',
          marginRight: '12px',
        }}
      >
        ←
      </button>
      <EditableTitle />
      <div className="header-actions">
        {/* Future: Save, Settings, etc. */}
      </div>
    </header>
  );
}
```

**Step 2: Test back navigation**

Navigate to `http://localhost:5173/` Double-click file icon to open editor Click back arrow button
Expected: Return to desktop

**Step 3: Commit**

```bash
git add app/src/components/header/header.tsx
git commit -m "feat(text-editor): add back navigation to desktop"
```

---

## Task 14: Create Sample Desktop Files

**Files:**

- Create: `app/scripts/createSampleFiles.ts`

**Step 1: Create sample files script**

Create `app/scripts/createSampleFiles.ts`:

```typescript
import { getVFSService } from '../src/lib/vfs-service';

async function createSampleFiles() {
  const vfs = getVFSService();

  await vfs.initialize('', '');

  // Create sample files
  const files = [
    {
      path: '/desktonk/Welcome.txt',
      content: {
        data: 'Welcome to Desktonk!\n\nThis is a browser-based desktop environment powered by TLDraw and Tonk VFS.',
        desktopMeta: {
          x: 50,
          y: 50,
        },
      },
    },
    {
      path: '/desktonk/README.md',
      content: {
        data: '# Desktonk\n\nDouble-click files to open them in apps.\nDrag files to reposition them.\n',
        desktopMeta: {
          x: 170,
          y: 50,
        },
      },
    },
    {
      path: '/desktonk/notes.json',
      content: {
        data: { title: 'My Notes', items: ['Task 1', 'Task 2'] },
        desktopMeta: {
          x: 290,
          y: 50,
        },
      },
    },
  ];

  for (const file of files) {
    try {
      await vfs.writeFile(file.path, { content: file.content }, true);
      console.log(`Created: ${file.path}`);
    } catch (error) {
      console.error(`Failed to create ${file.path}:`, error);
    }
  }

  console.log('Sample files created!');
}

// Run if this is the main module
if (import.meta.url === `file://${process.argv[1]}`) {
  createSampleFiles().catch(console.error);
}
```

**Step 2: Add script to package.json**

Modify `app/package.json` scripts section:

```json
{
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "create-samples": "bun scripts/createSampleFiles.ts",
    ...
  }
}
```

**Step 3: Run script to create samples**

```bash
cd app
bun run create-samples
```

Expected: Console shows "Created: /desktonk/..." for each file

**Step 4: Verify samples appear**

Navigate to `http://localhost:5173/` Expected: Three file icons appear (Welcome.txt, README.md,
notes.json)

**Step 5: Commit**

```bash
git add app/scripts/createSampleFiles.ts app/package.json
git commit -m "feat(desktop): add sample files creation script"
```

---

## Task 15: Final Testing and Documentation

**Files:**

- Create: `app/docs/DESKTOP.md`

**Step 1: Create desktop documentation**

Create `app/docs/DESKTOP.md`:

```markdown
# Desktonk Desktop

Browser-based desktop environment using TLDraw.

## Features

- **File Icons**: VFS files displayed as draggable icons
- **MIME Detection**: Automatic icon selection based on file type
- **App Launching**: Double-click to open files in apps
- **Position Persistence**: Icon positions saved to VFS
- **Auto-Layout**: New files auto-positioned in grid

## Usage

### Navigate Desktop

- Open: `http://localhost:5173/`
- The desktop shows all files from `/desktonk/` directory

### Open Files

- **Double-click** a file icon to open in default app
- Currently all text files open in text editor
- Back button returns to desktop

### Manage Files

- **Drag** icons to reposition (saves automatically)
- **Create** files via VFS (appear instantly)
- **Delete** files via VFS (disappear instantly)

## File Metadata

Files can include desktop metadata:

\`\`\`json { "data": "file content", "desktopMeta": { "x": 100, "y": 200, "icon": "🎨",
"appHandler": "text-editor" } } \`\`\`

## MIME to App Mapping

Default handlers in `utils/mimeResolver.ts`:

- `text/*` → text-editor
- `application/json` → text-editor

Override with `desktopMeta.appHandler`.

## Creating Files

Via console:

\`\`\`javascript const vfs = window.getVFSService(); await vfs.writeFile('/desktonk/myfile.txt', {
content: { data: 'Hello!' } }, true); \`\`\`

Via script:

\`\`\`bash bun run create-samples \`\`\`
```

**Step 2: Test complete workflow**

1. Navigate to `http://localhost:5173/`
2. Verify sample files appear
3. Drag a file icon to new position
4. Refresh page - verify position persisted
5. Double-click file - opens in text editor
6. Click back - returns to desktop
7. Create new file via console
8. Verify new file appears instantly

**Step 3: Commit documentation**

```bash
git add app/docs/DESKTOP.md
git commit -m "docs(desktop): add desktop usage documentation"
```

**Step 4: Final commit**

```bash
git commit --allow-empty -m "feat(desktonk): complete desktop environment implementation

- TLDraw-based desktop canvas
- Custom FileIcon shapes with MIME detection
- VFS directory sync with watchers
- Double-click app launching
- Position persistence
- Visual polish and animations
- Back navigation from apps
- Sample files script

Desktop displays VFS /desktonk/ files as draggable icons.
Text editor moved to /text-editor?file=... route.
Complete separation between desktop and apps."
```

---

## Verification Checklist

Before considering implementation complete, verify:

- [ ] Desktop route (`/`) renders TLDraw canvas
- [ ] Text editor route (`/text-editor?file=...`) loads and displays file
- [ ] File icons appear for all files in `/desktonk/`
- [ ] MIME type detection shows correct icons
- [ ] Double-click opens file in text editor
- [ ] Icon positions persist after drag
- [ ] New VFS files appear instantly
- [ ] Deleted VFS files disappear instantly
- [ ] Back button returns from editor to desktop
- [ ] Sample files script creates test files
- [ ] No console errors during normal operation
- [ ] TypeScript build completes without errors

## Next Steps (Future Enhancements)

1. **Image Viewer App**: Create `/image-viewer?file=...` for images
2. **File Operations**: Right-click context menu (rename, delete, properties)
3. **Keyboard Shortcuts**: Delete key, Cmd+O, etc.
4. **Desktop Background**: Custom wallpaper support
5. **Grid Snap**: Toggle for icon alignment
6. **Folders**: Directory navigation on desktop
7. **Search**: File search overlay
8. **Settings App**: Desktop preferences

---

**Implementation complete!** Desktop environment fully functional.

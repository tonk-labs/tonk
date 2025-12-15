# Desktonk Architecture

## Overview

Desktonk manages files on an infinite canvas. Three technologies power it:

- **TLDraw** - Renders draggable file icons on canvas
- **TipTap** - Edits rich text and markdown
- **Zustand** - Synchronizes state with VFS

The app runs inside the Tonk launcher and routes all file operations through a Service Worker-based Virtual File System (VFS).

## Package Structure

```mermaid
graph TB
    subgraph "packages/desktonk"
        main["main.tsx<br/><i>Entry point - VFS connection gate</i>"]
        router["Router.tsx<br/><i>Theme init, routing, basename</i>"]

        subgraph "src/features"
            desktop["desktop/<br/><i>TLDraw canvas + file icons</i>"]
            texteditor["text-editor/<br/><i>TipTap editor (standalone route)</i>"]
            editor["editor/<br/><i>Editor state store</i>"]
            chat["chat/<br/><i>Chat messages</i>"]
            presence["presence/<br/><i>User presence tracking</i>"]
            dock["dock/<br/><i>macOS-style dock</i>"]
            membersbar["members-bar/<br/><i>Online users sidebar</i>"]
        end

        subgraph "src/components"
            layout["layout/<br/><i>RootLayout (Dock + MembersBar)</i>"]
        end

        subgraph "src/lib"
            storeBuilder["storeBuilder.ts<br/><i>Zustand wrapper with persistence</i>"]
            middleware["middleware.ts<br/><i>VFS sync middleware</i>"]
            paths["paths.ts<br/><i>Filesystem path constants</i>"]
            featureFlags["featureFlags.ts<br/><i>Feature flag store</i>"]
        end

        subgraph "src/vfs-client"
            vfsService["vfs-service.ts<br/><i>Service Worker communication</i>"]
            vfsTypes["types.ts<br/><i>VFS type definitions</i>"]
        end

        hooks["src/hooks/<br/><i>Shared React hooks</i>"]
        scripts["scripts/bundleBuilder.ts<br/><i>Creates .tonk bundles</i>"]
        mprocs["mprocs.yaml<br/><i>Dev environment config</i>"]
        vite["vite.config.ts<br/><i>Build config (port 4001)</i>"]
    end
```

## Core Architectural Decisions

### 1. VFS Service Worker Architecture

A singleton VFSService sends all file operations to a Service Worker via `postMessage`. This enables offline-first behavior and CRDT-based synchronization.

**Key points:**
- App blocks rendering until VFS connects (see `main.tsx`)
- VFSService reconnects automatically and re-establishes watchers
- Requests timeout after 30 seconds

**Location:** `src/vfs-client/vfs-service.ts`

### 2. Three-Tier State Persistence

Each Zustand store chooses one persistence strategy:

| Strategy | Use Case | Example |
|----------|----------|---------|
| **VFS Sync** | Collaborative state shared across users | `presenceStore` |
| **localStorage** | Local UI preferences | `membersBarStore` |
| **None** | Session-only state | `editorStore` |

`StoreBuilder` handles all three with a consistent API.

**Location:** `src/lib/storeBuilder.ts`, `src/lib/middleware.ts`

### 3. Filesystem Hierarchy Standard (FHS)

All VFS paths follow a consistent hierarchy:

| Purpose | Path |
|---------|------|
| User files | `/desktonk/{filename}` |
| Position data | `/var/lib/desktonk/layout/{fileId}.json` |
| Thumbnails | `/var/lib/desktonk/thumbnails/{fileId}.png` |
| Canvas state | `/.state/desktop` |
| Presence | `/var/lib/desktonk/presence/users.json` |
| Chat | `/var/lib/desktonk/chat/messages.json` |

**Location:** `src/lib/paths.ts`

### 4. DesktopService Singleton Pattern

DesktopService manages file metadata and icon positions. It uses a custom singleton with publish/subscribe—**not** Zustand.

**Why a singleton?**
- Requires complex watcher lifecycle management
- Coordinates VFS operations with debouncing
- Depends on strict initialization order

**Key features:**
- Watches files, layouts, and positions for real-time updates
- Debounces position saves (500ms) and file changes (100ms)
- Integrates with React via `useDesktop` hook

**Location:** `src/features/desktop/services/DesktopService.ts`

### 5. Custom TLDraw Shape

File icons use a custom `FileIconUtil` shape:

- Shape ID format: `shape:file-icon:{fileId}`
- FileId = filename without extension (dotfiles keep the dot)
- Double-click opens the appropriate editor
- `useThumbnail` hook loads thumbnails asynchronously

**Location:** `src/features/desktop/shapes/FileIconUtil.tsx`

### 6. Theme Synchronization

Theme initializes before React renders to prevent flash:

1. IIFE in `Router.tsx` reads localStorage or system preference
2. Launcher sends theme changes via `postMessage`
3. Components listen for `theme-changed` custom event
4. TLDraw sets theme in `onMount` callback, not useEffect

**Location:** `src/Router.tsx:12-35`

## Data Flow

```mermaid
flowchart TB
    A[User Action] --> B[React Component]
    B --> C[Zustand Store / DesktopService]
    C --> D[VFS Sync Middleware<br/>2-second debounce]
    D --> E[VFSService.writeFile]
    E --> F[Service Worker<br/>postMessage]
    F --> G[Tonk Core CRDT]
    G --> H[Relay Sync<br/>other users]
```

## Build System

- **Vite** with `rolldown-vite` override for faster builds
- **mprocs** runs launcher + desktonk + relay in parallel for dev
- **Strict port 4001** for dev server
- `bun run bundle` creates `.tonk` files for deployment

# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this
repository.

## Project Overview

Latergram is a Tonk-based application that provides a dynamic visual development environment with
live editing capabilities. It uses a virtual file system (VFS) backed by Automerge for collaborative
state management and real-time updates.

## Architecture

### Core Technologies

- **Frontend**: React 19 with TypeScript, Vite for bundling
- **State Management**: Zustand stores, Automerge for CRDT-based collaboration
- **Virtual File System**: TonkCore with WebSocket sync to server
- **Styling**: Tailwind CSS via Twind runtime compilation
- **Server**: Express server with WebSocket support for Automerge sync

### Key Architectural Components

1. **Virtual File System (VFS)**
   - Managed through Web Worker (`src/tonk-worker.ts`) for non-blocking operations
   - All file operations (read/write/watch) go through VFS service (`src/services/vfs.ts`)
   - Supports real-time file watching and directory monitoring

2. **Dynamic Component System**
   - Components are stored as TypeScript files in VFS at `/src/components/`
   - Hot compilation via `HotCompiler.tsx` using TypeScript transpilation
   - Component registry manages available components for drag-and-drop

3. **Dynamic Views & Routing**
   - Views stored in VFS at `/src/views/`
   - `ViewRenderer` component dynamically loads and renders view files
   - Route-to-view mapping: URL path maps to `/src/views/{path}.tsx`

4. **Store Management**
   - Zustand stores defined in VFS at `/src/stores/`
   - Dynamic store compilation and registration
   - Shared state across components via store hooks

5. **TypeScript Validation**
   - Runtime TypeScript validation in `src/lib/typescript-validator.ts`
   - Compiles and validates code before execution
   - Provides diagnostics for error reporting

## Development Commands

```bash
# Start development (frontend + server)
pnpm dev

# Build for production
pnpm build

# Run in production mode
pnpm serve

# Clean build artifacts
pnpm clean

# Deploy to Tonk platform
pnpm deployment
```

### Server Commands (in `/server` directory)

```bash
cd server
pnpm dev  # Starts server on port 8081
```

## Important Patterns

### Working with VFS

- Always use VFS service methods, never direct file system access
- File paths in VFS start with `/` (e.g., `/src/components/Button.tsx`)
- File operations are async and go through Web Worker

### Component Development

- Components must export a default React component
- Available packages are injected via `contextBuilder.ts`
- Components have access to: React, Lucide icons, React Router, Zustand stores

### Store Integration

- Stores use Zustand pattern with TypeScript interfaces
- Access stores in components via: `const store = window.stores?.storeName`
- Stores are globally available after compilation

### TypeScript Compilation

- All TypeScript code is transpiled at runtime
- JSX is supported with React runtime
- Module resolution uses CommonJS pattern for dynamic execution

## Project Structure Notes

- `/src/components/` - Reusable UI components (stored in VFS)
- `/src/views/` - Page views mapped to routes (stored in VFS)
- `/src/stores/` - Zustand state stores (stored in VFS)
- `/src/services/` - Core services (VFS, stores, etc.)
- `/src/lib/` - Utilities (TypeScript validator, file validator)
- `/server/` - Express server for Automerge sync

## Current Branch Context

Working on branch: `ramram/feat/latergram-temp` Modified file: `src/lib/typescript-validator.ts` -
TypeScript validation functionality

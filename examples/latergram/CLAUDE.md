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
- **UI Library**: Chakra UI (primary component library)
- **State Management**: Zustand stores, Automerge for CRDT-based collaboration
- **Virtual File System**: TonkCore with WebSocket sync to server
- **Styling**: Chakra UI + Dual styling system with Twind runtime compilation and standard Tailwind CSS
- **Code Editor**: Monaco Editor for in-browser code editing
- **Server**: Express server with WebSocket support for Automerge sync

### Key Architectural Components

1. **Virtual File System (VFS)**
   - Managed through Web Worker (`src/tonk-worker.ts`) for non-blocking operations
   - All file operations (read/write/watch) go through VFS service (`src/services/vfs-service.ts`)
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

6. **Error Handling System**
   - Comprehensive error boundaries (`src/components/errors/`)
   - Inline error boundary creation for dynamic components
   - Error reporting to AI agent for debugging assistance

7. **AI Agent Integration**
   - Built-in AI chat interface (`src/components/chat/`)
   - Agent service for code assistance (`src/lib/agent/`)
   - Tool integration for VFS file operations
   - Real-time error reporting to agent

8. **Editor Integration**
   - Monaco Editor for code editing (`src/components/unified-editor/`)
   - Syntax highlighting and IntelliSense
   - Auto-save functionality
   - Preview pane for component visualization

## Development Commands

```bash
# Start development (frontend + server concurrently)
pnpm dev

# Build for production
pnpm build

# Run in production mode
pnpm serve

# Clean build artifacts
pnpm clean

# Deploy to Tonk platform
pnpm deployment

# Run linter
pnpm lint

# Run tests
pnpm test

# Watch tests
pnpm test:watch
```

### Server Commands (in `/server` directory)

```bash
cd server
pnpm dev  # Starts server on port 8081 with blank.tonk manifest
```

## Important Patterns

### Working with VFS

- Always use VFS service methods, never direct file system access
- File paths in VFS start with `/` (e.g., `/src/components/Button.tsx`)
- File operations are async and go through Web Worker

### Component Development

- Components must export a default React component
- Supports both `.tsx` and `.ts` file extensions
- **NO IMPORTS NEEDED** - All packages are globally available via `contextBuilder.ts`
- **Primary UI Library: Chakra UI** - Use Chakra UI components for all UI elements
- Components have access to:
  - **React & Hooks**: `React`, `useState`, `useEffect`, `useCallback`, `useMemo`, `useRef`, `useReducer`, `useContext`, `Fragment`
  - **Chakra UI v3**: All components (`Box`, `Button`, `Heading`, `Input`, `Modal`, etc.) and hooks (`useDisclosure`, `useBreakpoint`, etc.)
  - **Toast Notifications**: Global `toaster` utility for notifications (Chakra v3 API)
  - **React Router**: `Link`, `NavLink`, `useNavigate`, `useLocation`, `useParams`
  - **Zustand**: `create`, `sync` for store creation
  - **Custom Components & Stores**: All registered VFS components and stores

### Store Integration

- Stores use Zustand pattern with TypeScript interfaces
- Access stores in components via: `const store = window.stores?.storeName`
- Stores are globally available after compilation

### TypeScript Compilation

- All TypeScript code is transpiled at runtime
- JSX is supported with React runtime
- Module resolution uses CommonJS pattern for dynamic execution

## Project Structure

### VFS (Virtual File System) Locations

- `/src/components/` - Reusable UI components
- `/src/views/` - Page views mapped to routes
- `/src/stores/` - Zustand state stores

### Local File System

- `/src/services/` - Core services (VFS, user service)
- `/src/lib/` - Utilities (TypeScript validator, file validator, agent)
- `/src/components/` - UI infrastructure (error handling, chat, editor, file tree)
- `/server/` - Express server for Automerge sync

## Development Workflow

### When modifying VFS files

1. Use VFS service methods to read/write files
2. Changes auto-sync via Automerge to server
3. File watchers trigger hot recompilation

### When creating new components

1. Create TypeScript file in VFS at `/src/components/`
2. Export default React component
3. Component auto-registers for use in views

### When adding routes

1. Create view file at `/src/views/{route-name}.tsx`
2. View is automatically available at `/{route-name}` URL
3. Use React Router hooks for navigation

## Key Configuration Files

- `vite.config.ts` - Vite bundler configuration with PWA support, optimized chunking
- `tsconfig.json` - TypeScript compiler options (target: ESNext, module: ESNext)
- `mprocs.yaml` - Concurrent process configuration for dev server
- `package.json` - Dependencies and scripts
- `tailwind.config.cjs` - Tailwind CSS configuration
- `postcss.config.cjs` - PostCSS configuration for Tailwind
- `src/twind.config.ts` - Twind runtime styling configuration

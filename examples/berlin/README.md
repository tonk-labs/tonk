# Sprinkles 🧁

Unified dev environment for Tonk - runs host-web and relay together with a nice TUI.

## Features

### Editable Page Title

- Click the centered page title in the header to edit
- Press Enter to save or Escape to cancel
- Titles are automatically trimmed and limited to 100 characters
- Empty titles default to "Untitled"
- Title changes will sync in real-time across all connected clients via VFS (coming soon)
- Title is stored as metadata alongside document content

#### Storage Format

Document metadata is stored in the VFS as:

```json
{
  "content": {
    /* TipTap document JSON */
  },
  "metadata": {
    "title": "Document Title"
  }
}
```

This structure supports future metadata extensions (tags, description, author, etc.).

## What is this?

Sprinkles runs both the Tonk relay server and host-web frontend simultaneously using
[mprocs](https://github.com/pvolok/mprocs), giving you a split-screen terminal UI to monitor both
processes.

## Installation

```bash
# From the tonk monorepo root
pnpm install
```

This installs:

- `@tonk/relay` - WebSocket sync server, WASM server, bundle storage
- `@tonk/host-web` - Browser-based runtime for .tonk applications
- `mprocs` - Terminal process manager with TUI

## Usage

```bash
cd packages/sprinkles
pnpm run dev
```

This automatically:

1. **Builds dependencies** (`predev` lifecycle hook):
   - Builds WASM from Rust (`packages/core-js/build.sh`)
   - Builds service worker with `NODE_ENV=development`
   - Copies service worker to src for dev
2. **Starts both servers** via mprocs:
   - **relay** - Relay server on `http://localhost:8081`
   - **host-web** - Vite dev server on `http://localhost:4000`

### mprocs Controls

- **Tab** / **Shift+Tab** - Switch between processes
- **↑/↓** - Scroll output
- **r** - Restart selected process
- **s** - Stop selected process
- **a** - Add process
- **q** - Quit mprocs (stops all processes)

## Architecture

```
┌─────────────┐         ┌──────────────┐
│  host-web   │────────▶│    relay     │
│  (browser)  │  fetch  │  :8081       │
│             │◀────────│              │
│ - Serves UI │  wasm   │ - Serves WASM│
│ - VFS/SW    │ bundles │ - Automerge  │
│ - BIOS menu │   sync  │ - Bundles    │
└─────────────┘         └──────────────┘
```

## When to use this

- **Local development**: Working on Tonk core, relay, or host-web
- **Testing**: Want to test the full stack locally
- **Debugging**: Need to see both frontend and backend logs side-by-side

## Separate Repo Usage

This package is designed to work both in the monorepo and as a standalone repo:

**In monorepo** (current):

- Uses `workspace:*` dependencies
- pnpm creates symlinks in `node_modules/@tonk/*` pointing to local packages

**Standalone repo** (future):

```json
"dependencies": {
  "@tonk/host-web": "^0.0.1",
  "@tonk/relay": "^0.1.0"
}
```

Just publish `@tonk/host-web` and `@tonk/relay` to npm, update versions, and `pnpm install` will
pull from the registry.

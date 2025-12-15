# Desktonk

A desktop-like canvas application built with React, tldraw, and TipTap. Runs as a Tonk app inside
the launcher.

## Features

- **Desktop Canvas** - An infinite tldraw canvas with file/folder icons you can arrange
- **Text Editor** - TipTap-based rich text editor with markdown support and dual-mode editing
- **Dock** - Application launcher dock (Tinki app launcher)
- **Chat** - Integrated chat window
- **Presence** - Real-time user presence indicators
- **Members Bar** - Shows connected users
- **Dark Mode** - System-aware theme with manual toggle, syncs with launcher

## Architecture

```
src/
├── features/
│   ├── desktop/          # tldraw canvas with file icons
│   ├── text-editor/      # TipTap editor app (standalone route)
│   ├── editor/           # TipTap components and UI primitives
│   ├── dock/             # Application launcher dock
│   ├── chat/             # Chat window
│   ├── members-bar/      # Connected users display
│   └── presence/         # User presence tracking
├── contexts/             # React contexts (feature flags)
├── components/           # Shared UI components
├── hooks/                # Custom hooks (useVFS, useTheme, etc.)
├── stores/               # Zustand stores
├── lib/                  # Core utilities (middleware, storeBuilder, feature flags)
├── vfs-client/           # Virtual File System client service
├── utils/                # Utility functions (sample files)
├── styles/               # Shared styles
├── assets/               # Static assets
└── Router.tsx            # Route definitions
```

## Routes

| Path           | Component     | Description                    |
| -------------- | ------------- | ------------------------------ |
| `/`            | Desktop       | Main canvas view               |
| `/text-editor` | TextEditorApp | Rich text editor (with ?file=) |

## Development

```bash
# Start dev server with relay (uses mprocs)
bun run dev

# Build the app
bun run build

# Create a .tonk bundle
bun run bundle

# Lint (biome + oxlint)
bun run lint

# Format
bun run format

# Create sample files for testing
bun run create-samples
```

## Key Dependencies

- **tldraw** - Canvas framework for desktop icons
- **TipTap** - Rich text editor with markdown extension
- **@tonk/core** - Tonk VFS integration and CRDT sync
- **Zustand** - State management with immer middleware
- **React Router** - Client-side routing
- **Tailwind CSS** - Styling
- **Radix UI** - Accessible UI primitives

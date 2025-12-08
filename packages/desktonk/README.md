# Desktonk

A desktop-like canvas application built with React, tldraw, and TipTap. Runs as a Tonk app inside the launcher.

## Features

- **Desktop Canvas** - An infinite tldraw canvas with file/folder icons you can arrange
- **Text Editor** - TipTap-based rich text editor with collaborative editing support
- **Chat** - Integrated chat window
- **Presence** - Real-time user presence indicators
- **Dark Mode** - System-aware theme with manual toggle

## Architecture

```
src/
├── features/
│   ├── desktop/          # tldraw canvas with file icons
│   ├── text-editor/      # TipTap editor app
│   ├── editor/           # TipTap components and UI
│   ├── chat/             # Chat window
│   └── presence/         # User presence tracking
├── contexts/             # React contexts (feature flags)
├── components/           # Shared UI components
├── hooks/                # Custom hooks (useVFS, etc.)
└── Router.tsx            # Route definitions
```

## Routes

| Path           | Component       | Description                    |
| -------------- | --------------- | ------------------------------ |
| `/`            | Desktop         | Main canvas view               |
| `/text-editor` | TextEditorApp   | Rich text editor (with ?file=) |

## Development

```bash
# Start dev server with relay
bun run dev

# Build the app
bun run build

# Create a .tonk bundle
bun run bundle

# Lint and format
bun run lint
bun run format
```

## Dependencies

- **tldraw** - Canvas framework
- **TipTap** - Rich text editor
- **@tonk/core** - Tonk VFS integration
- **Zustand** - State management
- **React Router** - Routing

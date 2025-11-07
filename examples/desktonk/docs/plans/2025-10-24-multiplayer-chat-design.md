# Multiplayer Chat Feature Design

## Overview

A floating chat window for real-time multiplayer collaboration. Users send text messages, see typing
indicators, add emoji reactions, and view timestamps. Chat history persists globally across
documents with configurable limits.

## Architecture

### Hybrid Optimistic Updates

**Three layers:**

1. **UI Layer** - Floating window (React Portal + react-draggable)
2. **State Layer** - Zustand store with optimistic updates
3. **Persistence Layer** - VFS file as source of truth

**Sync flow:**

- User sends message → store updates immediately → write to VFS → broadcast via presence
- Other clients receive broadcast → update store → write to VFS
- On conflict (concurrent sends), VFS wins, store resyncs

### Integration Points

**VFS Integration:**

- File: `/chat-history.json`
- Format: `{ version: 1, messages: [...], config: { maxHistory: number } }`
- Loads on app start to hydrate store
- Writes after every message

**Presence Integration:**

- Broadcasts via existing presence system
- Event types: `chat:message`, `chat:typing`, `chat:reaction`
- Uses presence user info (displayName, avatar, color)
- No new infrastructure required

## Component Structure

Location: `features/chat/`

```
features/chat/
├── components/
│   ├── ChatWindow.tsx              Portal + drag container
│   ├── ChatHeader.tsx              Title + minimize/close buttons
│   ├── ChatMessageList.tsx         Scrollable with auto-scroll
│   ├── ChatMessage.tsx             Avatar + content + reactions
│   ├── ChatMessageReactions.tsx    Emoji picker + display
│   ├── ChatTypingIndicator.tsx     "User is typing..." badge
│   └── ChatInput.tsx               Input field + send button
├── stores/
│   └── chatStore.ts                Zustand store
├── hooks/
│   └── useChatSync.ts              VFS + presence sync
└── utils/
    └── chatHistory.ts              VFS read/write helpers
```

### Reused Components

**From `components/ui/button/`:**

- Send button (variant="default")
- Close/minimize (variant="ghost", size="icon-sm")

**From `features/editor/components/tiptap-ui-primitive/`:**

- `input/` - Message text input
- `tooltip/` - Timestamp hovers
- `badge/` - Typing indicators
- `card/` - Window container

**From `features/presence/`:**

- `getInitials()` utility
- User colors from presence system
- User info (name, id, displayName)

## Data Model

### Store State

```typescript
{
  messages: ChatMessage[]
  typingUsers: Set<string>
  windowState: {
    isOpen: boolean
    position: { x: number, y: number }
    size: { width: number, height: number }
  }
  config: {
    maxHistory: number  // 400 default, -1 = infinite
  }
}
```

### ChatMessage Type

```typescript
{
  id: string              // UUID
  userId: string          // From presence
  text: string
  timestamp: number       // Unix timestamp
  reactions: {
    emoji: string
    userIds: string[]
  }[]
}
```

## Features

### Messages

- Current user: right-aligned, `bg-primary`
- Other users: left-aligned, `bg-background` with border
- Avatar shows initials with color from presence
- Timestamp on each message

### Typing Indicators

- `onChange` debounced → broadcast `chat:typing`
- Auto-clears after 3s of inactivity
- Displays "User is typing..." at message list bottom

### Reactions

- Emoji picker for adding reactions
- Click emoji → add/remove user from reaction list
- Display reactions below each message

### History Management

- Configurable limit (default 400, -1 = infinite)
- Exceeding limit removes oldest messages
- Applies to both store and VFS

## UI Layout

**Floating window:**

- Draggable via header
- Resizable (optional, can add later)
- Toggled with button in toolbar
- Position persists in store

**Message layout:**

- Auto-scrolls to newest message
- Scrollable history
- Input fixed at bottom

## Styling

Uses existing design system:

- IBM Carbon colors
- Tailwind utilities
- `font-mono` throughout
- Follows existing component patterns

# Multiplayer Chat Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan
> task-by-task.

**Goal:** Build a floating chat window for real-time multiplayer collaboration with typing
indicators, emoji reactions, and persistent history.

**Architecture:** Hybrid optimistic updates with Zustand store, VFS persistence
(`/chat-history.json`), and presence system broadcasts for real-time sync.

**Tech Stack:** React, Zustand (via StoreBuilder), react-draggable, existing UI primitives (Button,
Input, Card, Tooltip, Badge)

---

## Task 1: Create Chat Store Foundation

**Files:**

- Create: `app/src/features/chat/stores/chatStore.ts`
- Create: `app/src/features/chat/types.ts`

**Step 1: Create types file**

Create `app/src/features/chat/types.ts`:

```typescript
export interface ChatMessage {
  id: string;
  userId: string;
  text: string;
  timestamp: number;
  reactions: ChatReaction[];
}

export interface ChatReaction {
  emoji: string;
  userIds: string[];
}

export interface WindowState {
  isOpen: boolean;
  position: { x: number; y: number };
  size: { width: number; height: number };
}

export interface ChatConfig {
  maxHistory: number; // 400 default, -1 = infinite
}
```

**Step 2: Create chat store**

Create `app/src/features/chat/stores/chatStore.ts`:

```typescript
import { StoreBuilder } from '../../../lib/storeBuilder';
import type { ChatMessage, WindowState, ChatConfig } from '../types';

interface ChatState {
  messages: ChatMessage[];
  typingUsers: Set<string>;
  windowState: WindowState;
  config: ChatConfig;
}

const initialState: ChatState = {
  messages: [],
  typingUsers: new Set(),
  windowState: {
    isOpen: false,
    position: { x: 100, y: 100 },
    size: { width: 400, height: 600 },
  },
  config: {
    maxHistory: 400,
  },
};

export const chatStore = StoreBuilder(initialState, {
  name: 'tonk-chat',
  version: 1,
  partialize: state => ({
    messages: state.messages,
    windowState: state.windowState,
    config: state.config,
    // Don't persist typingUsers (runtime only)
  }),
});

export const useChatStore = chatStore.useStore;
```

**Step 3: Add chat actions**

Add to `app/src/features/chat/stores/chatStore.ts` (append):

```typescript
const createChatActions = () => {
  const store = chatStore;

  return {
    /**
     * Add a message to the store (optimistic update)
     */
    addMessage: (message: ChatMessage) => {
      store.set(state => {
        state.messages.push(message);

        // Enforce history limit
        const { maxHistory } = state.config;
        if (maxHistory !== -1 && state.messages.length > maxHistory) {
          state.messages = state.messages.slice(-maxHistory);
        }
      });
    },

    /**
     * Set typing status for a user
     */
    setUserTyping: (userId: string, isTyping: boolean) => {
      store.set(state => {
        if (isTyping) {
          state.typingUsers.add(userId);
        } else {
          state.typingUsers.delete(userId);
        }
      });
    },

    /**
     * Toggle chat window open/close
     */
    toggleWindow: () => {
      store.set(state => {
        state.windowState.isOpen = !state.windowState.isOpen;
      });
    },

    /**
     * Update window position
     */
    updateWindowPosition: (x: number, y: number) => {
      store.set(state => {
        state.windowState.position = { x, y };
      });
    },

    /**
     * Add or remove reaction to a message
     */
    toggleReaction: (messageId: string, emoji: string, userId: string) => {
      store.set(state => {
        const message = state.messages.find(m => m.id === messageId);
        if (!message) return;

        const reaction = message.reactions.find(r => r.emoji === emoji);

        if (reaction) {
          const userIndex = reaction.userIds.indexOf(userId);
          if (userIndex > -1) {
            // Remove user from reaction
            reaction.userIds.splice(userIndex, 1);
            // Remove reaction if no users left
            if (reaction.userIds.length === 0) {
              message.reactions = message.reactions.filter(r => r.emoji !== emoji);
            }
          } else {
            // Add user to reaction
            reaction.userIds.push(userId);
          }
        } else {
          // Create new reaction
          message.reactions.push({ emoji, userIds: [userId] });
        }
      });
    },

    /**
     * Clear all messages
     */
    clearMessages: () => {
      store.set(state => {
        state.messages = [];
      });
    },
  };
};

export const useChat = chatStore.createFactory(createChatActions());
```

**Step 4: Commit**

```bash
git add app/src/features/chat/
git commit -m "feat(chat): add chat store with actions"
```

---

## Task 2: Create VFS Integration Utilities

**Files:**

- Create: `app/src/features/chat/utils/chatHistory.ts`

**Step 1: Create VFS chat history utilities**

Create `app/src/features/chat/utils/chatHistory.ts`:

```typescript
import type { ChatMessage, ChatConfig } from '../types';

interface ChatHistoryFile {
  version: number;
  messages: ChatMessage[];
  config: ChatConfig;
}

const CHAT_HISTORY_PATH = '/chat-history.json';

/**
 * Load chat history from VFS
 */
export async function loadChatHistory(): Promise<ChatHistoryFile | null> {
  try {
    // TODO: Replace with actual VFS read when available
    // For now, return null to initialize empty
    return null;
  } catch (error) {
    console.error('Failed to load chat history:', error);
    return null;
  }
}

/**
 * Save chat history to VFS
 */
export async function saveChatHistory(messages: ChatMessage[], config: ChatConfig): Promise<void> {
  const data: ChatHistoryFile = {
    version: 1,
    messages,
    config,
  };

  try {
    // TODO: Replace with actual VFS write when available
    console.log('Saving chat history to VFS:', CHAT_HISTORY_PATH, data);
  } catch (error) {
    console.error('Failed to save chat history:', error);
    throw error;
  }
}
```

**Step 2: Commit**

```bash
git add app/src/features/chat/utils/
git commit -m "feat(chat): add VFS integration utilities"
```

---

## Task 3: Create Chat Sync Hook

**Files:**

- Create: `app/src/features/chat/hooks/useChatSync.ts`

**Step 1: Create chat sync hook**

Create `app/src/features/chat/hooks/useChatSync.ts`:

```typescript
import { useEffect } from 'react';
import { useChat } from '../stores/chatStore';
import { loadChatHistory, saveChatHistory } from '../utils/chatHistory';

/**
 * Hook to sync chat store with VFS and presence system
 */
export function useChatSync() {
  const { messages, config } = useChat();

  // Load chat history on mount
  useEffect(() => {
    loadChatHistory().then(data => {
      if (data) {
        // TODO: Hydrate store with loaded data
        console.log('Loaded chat history:', data);
      }
    });
  }, []);

  // Save to VFS whenever messages change
  useEffect(() => {
    if (messages.length > 0) {
      saveChatHistory(messages, config);
    }
  }, [messages, config]);

  // TODO: Add presence system broadcast/receive integration
}
```

**Step 2: Commit**

```bash
git add app/src/features/chat/hooks/
git commit -m "feat(chat): add chat sync hook for VFS integration"
```

---

## Task 4: Create ChatMessage Component

**Files:**

- Create: `app/src/features/chat/components/ChatMessage.tsx`

**Step 1: Create ChatMessage component**

Create `app/src/features/chat/components/ChatMessage.tsx`:

```typescript
import type { ChatMessage as ChatMessageType } from '../types';
import { getInitials } from '../../presence/utils/userGeneration';
import { usePresence } from '../../presence/stores/presenceStore';
import { Tooltip, TooltipTrigger, TooltipContent } from '../../editor/components/tiptap-ui-primitive/tooltip/tooltip';

interface ChatMessageProps {
  message: ChatMessageType;
  isCurrentUser: boolean;
}

export function ChatMessage({ message, isCurrentUser }: ChatMessageProps) {
  const { getActiveUsers } = usePresence();
  const users = getActiveUsers();
  const user = users.find((u) => u.id === message.userId);

  const userName = user?.name || 'Unknown';
  const userColor = user?.color || '#888888';
  const userInitials = getInitials(userName);

  const formattedTime = new Date(message.timestamp).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  });

  return (
    <div
      className={`flex gap-2 mb-3 ${isCurrentUser ? 'flex-row-reverse' : 'flex-row'}`}
    >
      {/* Avatar */}
      <Tooltip placement="top" delay={300}>
        <TooltipTrigger asChild>
          <div
            className="w-8 h-8 rounded-full flex items-center justify-center text-white text-xs font-medium shrink-0"
            style={{ backgroundColor: userColor }}
          >
            {userInitials}
          </div>
        </TooltipTrigger>
        <TooltipContent>{userName}</TooltipContent>
      </Tooltip>

      {/* Message content */}
      <div className={`flex flex-col gap-1 max-w-[70%] ${isCurrentUser ? 'items-end' : 'items-start'}`}>
        <div
          className={`rounded-lg px-3 py-2 ${
            isCurrentUser
              ? 'bg-primary text-primary-foreground'
              : 'bg-background border border-border'
          }`}
        >
          <p className="text-sm font-mono whitespace-pre-wrap break-words">
            {message.text}
          </p>
        </div>

        {/* Timestamp */}
        <Tooltip placement="top" delay={300}>
          <TooltipTrigger asChild>
            <span className="text-xs text-muted-foreground font-mono cursor-default">
              {formattedTime}
            </span>
          </TooltipTrigger>
          <TooltipContent>
            {new Date(message.timestamp).toLocaleString()}
          </TooltipContent>
        </Tooltip>

        {/* Reactions (placeholder) */}
        {message.reactions.length > 0 && (
          <div className="flex gap-1 flex-wrap">
            {message.reactions.map((reaction) => (
              <span
                key={reaction.emoji}
                className="text-xs bg-accent px-2 py-1 rounded-full font-mono"
              >
                {reaction.emoji} {reaction.userIds.length}
              </span>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
```

**Step 2: Commit**

```bash
git add app/src/features/chat/components/ChatMessage.tsx
git commit -m "feat(chat): add ChatMessage component"
```

---

## Task 5: Create ChatTypingIndicator Component

**Files:**

- Create: `app/src/features/chat/components/ChatTypingIndicator.tsx`

**Step 1: Create typing indicator component**

Create `app/src/features/chat/components/ChatTypingIndicator.tsx`:

```typescript
import { Badge } from '../../editor/components/tiptap-ui-primitive/badge/badge';
import { usePresence } from '../../presence/stores/presenceStore';
import { useChat } from '../stores/chatStore';

export function ChatTypingIndicator() {
  const { typingUsers } = useChat();
  const { getActiveUsers } = usePresence();
  const users = getActiveUsers();

  if (typingUsers.size === 0) return null;

  const typingUserNames = Array.from(typingUsers)
    .map((userId) => {
      const user = users.find((u) => u.id === userId);
      return user?.name || 'Unknown';
    })
    .slice(0, 3); // Show max 3 names

  const displayText =
    typingUsers.size === 1
      ? `${typingUserNames[0]} is typing...`
      : typingUsers.size === 2
      ? `${typingUserNames[0]} and ${typingUserNames[1]} are typing...`
      : `${typingUserNames.slice(0, 2).join(', ')} and ${typingUsers.size - 2} others are typing...`;

  return (
    <div className="px-3 py-2">
      <Badge variant="secondary" className="text-xs font-mono">
        {displayText}
      </Badge>
    </div>
  );
}
```

**Step 2: Commit**

```bash
git add app/src/features/chat/components/ChatTypingIndicator.tsx
git commit -m "feat(chat): add typing indicator component"
```

---

## Task 6: Create ChatInput Component

**Files:**

- Create: `app/src/features/chat/components/ChatInput.tsx`

**Step 1: Create chat input component**

Create `app/src/features/chat/components/ChatInput.tsx`:

```typescript
import { useState, useCallback, useRef, useEffect } from 'react';
import { Button } from '../../../components/ui/button/button';
import { Input } from '../../editor/components/tiptap-ui-primitive/input/input';
import { useChat } from '../stores/chatStore';
import { usePresence } from '../../presence/stores/presenceStore';

export function ChatInput() {
  const [text, setText] = useState('');
  const { addMessage, setUserTyping } = useChat();
  const { users, currentUserId } = usePresence();
  const typingTimeoutRef = useRef<NodeJS.Timeout | null>(null);

  const handleTyping = useCallback(() => {
    if (!currentUserId) return;

    // Set typing to true
    setUserTyping(currentUserId, true);

    // Clear existing timeout
    if (typingTimeoutRef.current) {
      clearTimeout(typingTimeoutRef.current);
    }

    // Auto-clear typing after 3 seconds
    typingTimeoutRef.current = setTimeout(() => {
      setUserTyping(currentUserId, false);
    }, 3000);
  }, [currentUserId, setUserTyping]);

  const handleSend = useCallback(() => {
    if (!text.trim() || !currentUserId) return;

    const message = {
      id: crypto.randomUUID(),
      userId: currentUserId,
      text: text.trim(),
      timestamp: Date.now(),
      reactions: [],
    };

    addMessage(message);
    setText('');
    setUserTyping(currentUserId, false);

    // Clear typing timeout
    if (typingTimeoutRef.current) {
      clearTimeout(typingTimeoutRef.current);
      typingTimeoutRef.current = null;
    }

    // TODO: Broadcast message via presence system
  }, [text, currentUserId, addMessage, setUserTyping]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend]
  );

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (typingTimeoutRef.current) {
        clearTimeout(typingTimeoutRef.current);
      }
    };
  }, []);

  return (
    <div className="p-3 border-t border-border bg-background">
      <div className="flex gap-2">
        <Input
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            handleTyping();
          }}
          onKeyDown={handleKeyDown}
          placeholder="Type a message..."
          className="flex-1 font-mono"
        />
        <Button
          onClick={handleSend}
          disabled={!text.trim()}
          size="default"
        >
          Send
        </Button>
      </div>
    </div>
  );
}
```

**Step 2: Commit**

```bash
git add app/src/features/chat/components/ChatInput.tsx
git commit -m "feat(chat): add chat input with typing detection"
```

---

## Task 7: Create ChatMessageList Component

**Files:**

- Create: `app/src/features/chat/components/ChatMessageList.tsx`

**Step 1: Create message list component**

Create `app/src/features/chat/components/ChatMessageList.tsx`:

```typescript
import { useEffect, useRef } from 'react';
import { useChat } from '../stores/chatStore';
import { usePresence } from '../../presence/stores/presenceStore';
import { ChatMessage } from './ChatMessage';
import { ChatTypingIndicator } from './ChatTypingIndicator';

export function ChatMessageList() {
  const { messages } = useChat();
  const { currentUserId } = usePresence();
  const scrollRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when new messages arrive
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  return (
    <div
      ref={scrollRef}
      className="flex-1 overflow-y-auto p-3 bg-background"
    >
      {messages.length === 0 ? (
        <div className="flex items-center justify-center h-full text-muted-foreground font-mono text-sm">
          No messages yet. Start the conversation!
        </div>
      ) : (
        messages.map((message) => (
          <ChatMessage
            key={message.id}
            message={message}
            isCurrentUser={message.userId === currentUserId}
          />
        ))
      )}
      <ChatTypingIndicator />
    </div>
  );
}
```

**Step 2: Commit**

```bash
git add app/src/features/chat/components/ChatMessageList.tsx
git commit -m "feat(chat): add message list with auto-scroll"
```

---

## Task 8: Create ChatHeader Component

**Files:**

- Create: `app/src/features/chat/components/ChatHeader.tsx`

**Step 1: Create chat header component**

Create `app/src/features/chat/components/ChatHeader.tsx`:

```typescript
import { Button } from '../../../components/ui/button/button';
import { useChat } from '../stores/chatStore';

export function ChatHeader() {
  const { toggleWindow } = useChat();

  return (
    <div
      className="px-4 py-3 border-b border-border bg-background cursor-move select-none"
      data-drag-handle="true"
    >
      <div className="flex items-center justify-between">
        <h3 className="font-mono font-medium text-sm">Chat</h3>
        <div className="flex gap-1">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={toggleWindow}
            aria-label="Close chat"
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
            >
              <line x1="4" y1="4" x2="12" y2="12" />
              <line x1="12" y1="4" x2="4" y2="12" />
            </svg>
          </Button>
        </div>
      </div>
    </div>
  );
}
```

**Step 2: Commit**

```bash
git add app/src/features/chat/components/ChatHeader.tsx
git commit -m "feat(chat): add chat header with close button"
```

---

## Task 9: Create ChatWindow Component with Draggable

**Files:**

- Create: `app/src/features/chat/components/ChatWindow.tsx`

**Step 1: Install react-draggable**

```bash
bun add react-draggable
bun add -D @types/react-draggable
```

**Step 2: Create chat window component**

Create `app/src/features/chat/components/ChatWindow.tsx`:

```typescript
import { createPortal } from 'react-dom';
import Draggable from 'react-draggable';
import { useChat } from '../stores/chatStore';
import { useChatSync } from '../hooks/useChatSync';
import { ChatHeader } from './ChatHeader';
import { ChatMessageList } from './ChatMessageList';
import { ChatInput } from './ChatInput';
import { Card } from '../../editor/components/tiptap-ui-primitive/card/card';

export function ChatWindow() {
  const { windowState, updateWindowPosition } = useChat();

  // Sync with VFS and presence
  useChatSync();

  if (!windowState.isOpen) return null;

  const handleDragStop = (_e: any, data: { x: number; y: number }) => {
    updateWindowPosition(data.x, data.y);
  };

  return createPortal(
    <Draggable
      handle="[data-drag-handle='true']"
      position={windowState.position}
      onStop={handleDragStop}
      bounds="parent"
    >
      <div
        style={{
          position: 'fixed',
          width: windowState.size.width,
          height: windowState.size.height,
          zIndex: 1000,
        }}
      >
        <Card className="h-full flex flex-col shadow-lg border border-border">
          <ChatHeader />
          <ChatMessageList />
          <ChatInput />
        </Card>
      </div>
    </Draggable>,
    document.body
  );
}
```

**Step 3: Commit**

```bash
git add app/src/features/chat/components/ChatWindow.tsx package.json
git commit -m "feat(chat): add floating draggable chat window"
```

---

## Task 10: Create Chat Feature Export

**Files:**

- Create: `app/src/features/chat/index.ts`

**Step 1: Create feature export file**

Create `app/src/features/chat/index.ts`:

```typescript
export { ChatWindow } from './components/ChatWindow';
export { useChat } from './stores/chatStore';
export type { ChatMessage, ChatReaction, WindowState, ChatConfig } from './types';
```

**Step 2: Commit**

```bash
git add app/src/features/chat/index.ts
git commit -m "feat(chat): add feature exports"
```

---

## Task 11: Add Chat Toggle Button to Toolbar

**Files:**

- Modify: `app/src/App.tsx` (or wherever the main toolbar is)

**Step 1: Import and add ChatWindow to app**

Find your main App component and add the ChatWindow:

```typescript
import { ChatWindow } from './features/chat';
import { useChat } from './features/chat';

// Inside your App component, add:
function App() {
  const { toggleWindow, windowState } = useChat();

  return (
    <>
      {/* Existing app content */}

      {/* Add chat toggle button to toolbar */}
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={toggleWindow}
        aria-label="Toggle chat"
        className="fixed bottom-4 right-4"
      >
        <svg
          width="20"
          height="20"
          viewBox="0 0 20 20"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
        >
          <path d="M2 2h16v12H6l-4 4V2z" />
        </svg>
        {windowState.isOpen && <span className="sr-only">Close chat</span>}
        {!windowState.isOpen && <span className="sr-only">Open chat</span>}
      </Button>

      {/* Chat window */}
      <ChatWindow />
    </>
  );
}
```

**Step 2: Test the chat**

```bash
bun run dev
```

Open the app, click the chat button, verify:

- Window opens and is draggable
- Can type messages
- Messages appear with avatar and timestamp
- Enter key sends message

**Step 3: Commit**

```bash
git add app/src/App.tsx
git commit -m "feat(chat): integrate chat window into app"
```

---

## Task 12: Add Presence Broadcast Integration (TODO)

**Files:**

- Modify: `app/src/features/chat/hooks/useChatSync.ts`
- Modify: `app/src/features/chat/components/ChatInput.tsx`

**Note:** This task requires understanding of the presence broadcast mechanism. Once the presence
system's broadcast API is available:

1. In `useChatSync.ts`, add listeners for incoming `chat:message` and `chat:typing` events
2. In `ChatInput.tsx`, broadcast `chat:message` when sending
3. Broadcast `chat:typing` status changes

**Placeholder implementation in `useChatSync.ts`:**

```typescript
// Add to useChatSync hook:
useEffect(() => {
  // TODO: Subscribe to presence broadcasts
  // presenceSystem.on('chat:message', (message) => {
  //   addMessage(message);
  // });
  //
  // presenceSystem.on('chat:typing', ({ userId, isTyping }) => {
  //   setUserTyping(userId, isTyping);
  // });
  // return () => {
  //   presenceSystem.off('chat:message');
  //   presenceSystem.off('chat:typing');
  // };
}, []);
```

---

## Task 13: Add Message Reactions UI (Optional Enhancement)

**Files:**

- Create: `app/src/features/chat/components/ChatMessageReactions.tsx`
- Modify: `app/src/features/chat/components/ChatMessage.tsx`

**Step 1: Create emoji picker component**

Create `app/src/features/chat/components/ChatMessageReactions.tsx`:

```typescript
import { useState } from 'react';
import { Button } from '../../../components/ui/button/button';
import { useChat } from '../stores/chatStore';
import { usePresence } from '../../presence/stores/presenceStore';

interface ChatMessageReactionsProps {
  messageId: string;
  reactions: { emoji: string; userIds: string[] }[];
}

const QUICK_REACTIONS = ['👍', '❤️', '😂', '🎉', '🤔'];

export function ChatMessageReactions({
  messageId,
  reactions,
}: ChatMessageReactionsProps) {
  const [showPicker, setShowPicker] = useState(false);
  const { toggleReaction } = useChat();
  const { currentUserId } = usePresence();

  if (!currentUserId) return null;

  const handleReaction = (emoji: string) => {
    toggleReaction(messageId, emoji, currentUserId);
    setShowPicker(false);
  };

  return (
    <div className="relative">
      <div className="flex gap-1 items-center flex-wrap">
        {reactions.map((reaction) => {
          const hasReacted = reaction.userIds.includes(currentUserId);
          return (
            <button
              key={reaction.emoji}
              onClick={() => handleReaction(reaction.emoji)}
              className={`text-xs px-2 py-1 rounded-full font-mono transition-colors ${
                hasReacted
                  ? 'bg-primary text-primary-foreground'
                  : 'bg-accent hover:bg-accent/80'
              }`}
            >
              {reaction.emoji} {reaction.userIds.length}
            </button>
          );
        })}

        <Button
          variant="ghost"
          size="icon-sm"
          onClick={() => setShowPicker(!showPicker)}
          className="h-6 w-6"
        >
          +
        </Button>
      </div>

      {showPicker && (
        <div className="absolute bottom-full left-0 mb-1 p-2 bg-background border border-border rounded-lg shadow-lg flex gap-1">
          {QUICK_REACTIONS.map((emoji) => (
            <button
              key={emoji}
              onClick={() => handleReaction(emoji)}
              className="text-lg hover:bg-accent rounded p-1"
            >
              {emoji}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
```

**Step 2: Integrate reactions into ChatMessage**

Modify `app/src/features/chat/components/ChatMessage.tsx`:

Replace the placeholder reactions section with:

```typescript
import { ChatMessageReactions } from './ChatMessageReactions';

// In the component, replace the reactions section:
{message.reactions.length > 0 || isCurrentUser ? (
  <ChatMessageReactions
    messageId={message.id}
    reactions={message.reactions}
  />
) : null}
```

**Step 3: Commit**

```bash
git add app/src/features/chat/components/ChatMessageReactions.tsx app/src/features/chat/components/ChatMessage.tsx
git commit -m "feat(chat): add emoji reactions to messages"
```

---

## Testing Checklist

- [ ] Chat window opens and closes
- [ ] Window is draggable
- [ ] Messages send and appear in list
- [ ] Current user messages appear on right
- [ ] Other user messages appear on left (test with multiple browser tabs)
- [ ] Typing indicators show when typing
- [ ] Typing indicators clear after 3 seconds
- [ ] Messages persist in localStorage
- [ ] History limit enforced (test by sending >400 messages)
- [ ] Reactions can be added/removed
- [ ] Timestamps display correctly

---

## Future Enhancements

1. **VFS Integration:** Replace TODO comments with actual VFS API calls once available
2. **Presence Broadcasting:** Implement real-time message sync via presence system
3. **Read Receipts:** Show which users have seen messages
4. **Message Editing:** Allow users to edit their own messages
5. **Message Deletion:** Allow users to delete their own messages
6. **File/Image Sharing:** Support uploading and sharing files
7. **@Mentions:** Add user mention autocomplete
8. **Search:** Add search functionality for message history
9. **Notifications:** Desktop notifications for new messages when window closed
10. **Sound Effects:** Optional sound for incoming messages

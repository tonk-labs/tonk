# AgentChat Refactoring Plan

## Objective

Transform the 467-line monolithic AgentChat.tsx into a maintainable, component-based architecture
where:

1. Chat service runs independently in background
2. Frontend purely visualizes backend state
3. Components follow SRP with <400 LOC per file

## Core Principles

- **KISS**: Keep it simple, no over-engineering
- **SRP**: Single Responsibility Principle for each component
- **DRY**: Reuse functions and components
- **YAGNI**: Build only what's needed now

## Architecture

### Phase 1: State Management Layer

#### 1.1 Chat Store (`/src/stores/chat-store.ts`) - ~150 LOC

```typescript
// Zustand store that syncs with agent service
interface ChatStore {
  messages: ChatMessage[];
  isLoading: boolean;
  error: string | null;
  isReady: boolean;

  // Actions
  syncWithService: () => void;
  setLoading: (loading: boolean) => void;
  setError: (error: string | null) => void;
}
```

- Polls agent service for updates
- Maintains UI state independent of component lifecycle
- No business logic, only state synchronization

#### 1.2 Enhanced Agent Service Hook (`/src/lib/agent/use-agent-store.ts`) - ~100 LOC

```typescript
// Bridge between store and service
function useAgentStore() {
  // Subscribe to store
  // Handle background sync
  // Return stable references
}
```

### Phase 2: Component Breakdown

#### 2.1 Message Components (~50-100 LOC each)

**`/src/components/chat/ChatMessage.tsx`**

```typescript
// Renders single message with avatar and content
interface ChatMessageProps {
  message: ChatMessage;
  onEdit?: (id: string) => void;
  onDelete?: (id: string) => void;
}
```

**`/src/components/chat/ChatMessageContent.tsx`**

```typescript
// Handles markdown/plain text rendering
interface ChatMessageContentProps {
  content: string;
  role: 'user' | 'assistant';
}
```

**`/src/components/chat/ChatMessageActions.tsx`**

```typescript
// Edit and delete buttons for messages
interface ChatMessageActionsProps {
  messageId: string;
  onEdit: () => void;
  onDelete: () => void;
}
```

#### 2.2 Input Components (~50 LOC each)

**`/src/components/chat/ChatInput.tsx`**

```typescript
// Text input with keyboard handling
interface ChatInputProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  disabled?: boolean;
}
```

**`/src/components/chat/ChatInputBar.tsx`**

```typescript
// Combines input + send button
interface ChatInputBarProps {
  onSendMessage: (text: string) => void;
  isLoading: boolean;
}
```

#### 2.3 Status Components (~30-50 LOC each)

**`/src/components/chat/ChatHeader.tsx`**

```typescript
// Title, status badge, clear button
interface ChatHeaderProps {
  isReady: boolean;
  messageCount: number;
  onClear: () => void;
}
```

**`/src/components/chat/ChatLoadingDots.tsx`**

```typescript
// Animated typing indicator
// No props needed - pure presentation
```

**`/src/components/chat/ChatErrorBar.tsx`**

```typescript
// Error message display
interface ChatErrorBarProps {
  error: string;
}
```

#### 2.4 Editor Components (~100 LOC each)

**`/src/components/chat/MessageEditor.tsx`**

```typescript
// Inline message editing
interface MessageEditorProps {
  initialContent: string;
  onSave: (content: string) => void;
  onCancel: () => void;
  showWarning?: boolean;
}
```

#### 2.5 Tool Display (~75 LOC)

**`/src/components/chat/ToolCallDetails.tsx`**

```typescript
// Collapsible tool call display
interface ToolCallDetailsProps {
  toolCalls: ToolCall[];
}
```

### Phase 3: Utilities

#### 3.1 Types (`/src/components/chat/types.ts`) - ~50 LOC

```typescript
export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
  toolCalls?: ToolCall[];
}

export interface ToolCall {
  id: string;
  name: string;
  args: any;
  result: any;
}
```

#### 3.2 Helpers (`/src/components/chat/helpers.ts`) - ~50 LOC

```typescript
export function formatTimestamp(ts: number): string;
export function truncateContent(content: string, maxLen: number): string;
export function detectMarkdown(content: string): boolean;
```

#### 3.3 Hooks (`/src/components/chat/hooks.ts`) - ~100 LOC

```typescript
export function useScrollToBottom(deps: any[]);
export function useMessageEdit(onUpdate: Function);
export function useKeyboardSubmit(onSubmit: Function);
```

### Phase 4: Main Component Refactor

**`/src/views/AgentChat.tsx`** - ~150 LOC

```typescript
// Orchestrator component
function AgentChat() {
  const store = useAgentStore()
  const { scrollRef } = useScrollToBottom([store.messages])

  return (
    <div className="chat-container">
      <ChatHeader {...} />
      {store.error && <ChatErrorBar error={store.error} />}

      <div className="messages-area" ref={scrollRef}>
        {store.messages.map(msg => (
          <ChatMessage key={msg.id} message={msg} />
        ))}
        {store.isLoading && <ChatLoadingDots />}
      </div>

      <ChatInputBar
        onSendMessage={store.sendMessage}
        isLoading={store.isLoading}
      />
    </div>
  )
}
```

## Implementation Order

### Step 1: Core Infrastructure (2 files)

1. Create chat-store.ts with Zustand store
2. Create use-agent-store.ts hook

### Step 2: Basic Components (6 files)

1. ChatMessage.tsx
2. ChatMessageContent.tsx
3. ChatInput.tsx
4. ChatInputBar.tsx
5. ChatHeader.tsx
6. ChatLoadingDots.tsx

### Step 3: Enhanced Features (4 files)

1. MessageEditor.tsx
2. ChatMessageActions.tsx
3. ToolCallDetails.tsx
4. ChatErrorBar.tsx

### Step 4: Utilities (3 files)

1. types.ts
2. helpers.ts
3. hooks.ts

### Step 5: Integration (1 file)

1. Refactor AgentChat.tsx to use new components

## File Structure

```
src/
├── stores/
│   └── chat-store.ts (150 LOC)
├── lib/agent/
│   └── use-agent-store.ts (100 LOC)
├── components/chat/
│   ├── ChatMessage.tsx (100 LOC)
│   ├── ChatMessageContent.tsx (80 LOC)
│   ├── ChatMessageActions.tsx (50 LOC)
│   ├── ChatInput.tsx (50 LOC)
│   ├── ChatInputBar.tsx (50 LOC)
│   ├── ChatHeader.tsx (50 LOC)
│   ├── ChatLoadingDots.tsx (30 LOC)
│   ├── ChatErrorBar.tsx (30 LOC)
│   ├── MessageEditor.tsx (100 LOC)
│   ├── ToolCallDetails.tsx (75 LOC)
│   ├── types.ts (50 LOC)
│   ├── helpers.ts (50 LOC)
│   └── hooks.ts (100 LOC)
└── views/
    └── AgentChat.tsx (150 LOC) - refactored
```

## Success Metrics

- [ ] No file exceeds 400 LOC
- [ ] Each component has single responsibility
- [ ] Chat continues when component unmounted
- [ ] Functions are reused across components
- [ ] No placeholder/TODO code
- [ ] Clean separation of concerns

## Non-Goals (Out of Scope)

- Animation libraries
- Virtual scrolling (unless perf issue)
- Message search/filtering
- Export functionality
- Theming system
- i18n/localization
- Advanced markdown features
- File uploads
- Voice input

## Testing Strategy

Each component should be testable in isolation:

1. Props in → Render out
2. No direct service calls from components
3. Store handles all state management
4. Mock store for component tests

## Performance Considerations

- Use React.memo only where measured impact
- Debounce input (already exists)
- Lazy load markdown renderer
- Keep tool details collapsed by default

## Migration Path

1. Build new components alongside old code
2. Test each component individually
3. Switch AgentChat to use new components
4. Remove old code
5. Verify background operation

## Review Checklist

- [ ] Each file under 400 LOC?
- [ ] Functions extracted and reused?
- [ ] Single responsibility per component?
- [ ] No over-engineering?
- [ ] Background chat working?
- [ ] Type safety throughout?
- [ ] Clean imports/exports?

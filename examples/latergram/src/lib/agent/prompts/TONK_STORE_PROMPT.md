# Tonk Store Creation Guidelines for LLM Agents

## Critical Store Format Requirements

When creating Zustand stores in the Tonk environment, you MUST follow this exact format. These
stores are built around a bespoke HMR (Hot Module Replacement) compilation solution that requires
strict adherence to this pattern.

Stores are stored as `tsx` files in `/src/stores/` and are automatically compiled and hot-reloaded.

The data of the store should be persisted as JSON in the same folder, ideally with the same name as
the store. You can read this to get the content of the store.

## Mandatory Store Structure

```typescript
// REQUIRED: This exact comment must appear at the top
// create and sync are available in the compilation context

// 1. Define TypeScript interfaces for state and actions
interface YourStateInterface {
  // State properties
  property1: type;
  property2: type;

  // Action methods
  actionMethod: (param: type) => void;
  asyncMethod: (param: type) => Promise<void>;
  getterMethod: () => type;
}

// 2. Create the store using this EXACT pattern
const useYourStore = create<YourStateInterface>()(
  sync(
    (set, get) => ({
      // Initial state values
      property1: initialValue,
      property2: initialValue,

      // Action implementations
      actionMethod: param => {
        set(state => ({
          // Update state immutably
          property1: newValue,
        }));
      },

      // Async actions are allowed
      asyncMethod: async param => {
        // Can perform async operations
        const result = await someAsyncOperation();
        set({ property1: result });
      },

      // Can use get() to access current state
      getterMethod: () => {
        const currentState = get();
        return currentState.property1;
      },
    }),
    // REQUIRED: sync configuration with path
    { path: '/src/stores/your-store-name.json' }
  )
);

// REQUIRED: Default export
export default useYourStore;
```

## Critical Rules

### 1. Import Comment

- **MANDATORY**: The comment `// create and sync are available in the compilation context` must be
  the first line
- This tells the HMR system that `create` and `sync` are globally available
- DO NOT import these from any package

### 2. Store Creation Pattern

- **MUST** use `create<Interface>()()` with double parentheses
- **MUST** wrap the store implementation with `sync()`
- **MUST** provide both `set` and `get` parameters to the store function

### 3. Sync Configuration & JSON Persistence

- **REQUIRED**: Include `{ path: '/src/stores/store-name.json' }` as the second argument to sync
- Path should follow the pattern: `/src/stores/[store-name].json`
- Use kebab-case for the JSON filename
- **IMPORTANT**: This JSON file is where the store's state is persisted
- The JSON file enables:
  - State persistence across HMR (Hot Module Replacement) reloads
  - State sharing between development sessions
  - State debugging by inspecting the JSON file
- The sync middleware automatically:
  - Saves state changes to the JSON file
  - Loads initial state from the JSON file if it exists
  - Handles serialization/deserialization of the state

### 4. State Updates

- Use `set((state) => ({ ... }))` for partial updates based on current state
- Use `set({ ... })` for direct updates
- Always update state immutably - spread existing objects/arrays when modifying

### 5. Export Pattern

- **MUST** use default export
- Store hook name should follow pattern: `use[StoreName]Store`

## Common Patterns

### ID Generation

```typescript
// For unique IDs, use this pattern:
const id = Date.now().toString() + Math.random().toString(36).substr(2, 9);
```

### Array Updates

```typescript
// Adding items
set(state => ({
  items: [...state.items, newItem],
}));

// Removing items
set(state => ({
  items: state.items.filter(item => item.id !== idToRemove),
}));

// Updating items
set(state => ({
  items: state.items.map(item => (item.id === idToUpdate ? { ...item, ...updates } : item)),
}));
```

### Async Actions

```typescript
asyncAction: async params => {
  // Set loading state
  set({ isLoading: true });

  try {
    const result = await someAsyncCall(params);
    set({
      data: result,
      isLoading: false,
      error: null,
    });
  } catch (error) {
    set({
      isLoading: false,
      error: error.message,
    });
  }
};
```

### Using get() to Access Current State

```typescript
actionWithCurrentState: () => {
  const { property1, property2 } = get();
  // Use current state values
  if (property1 > 10) {
    set({ property2: 'high' });
  }
};
```

## File Validation Rules

The Tonk system enforces these validation rules:

1. Files must be syntactically valid TypeScript/JavaScript
2. No unused variables (will cause validation errors)
3. Use `const` instead of `let` where possible
4. Proper TypeScript typing (avoid `any` when possible)
5. Semicolons required at statement ends
6. Single quotes for strings
7. 2-space indentation

## Example: Complete Chat Store

```typescript
// create and sync are available in the compilation context
interface ChatMessage {
  id: string;
  name: string;
  text: string;
  timestamp: Date;
}

interface ChatState {
  messages: ChatMessage[];
  isTyping: boolean;
  currentUser: string;
  addMessage: (name: string, text: string) => void;
  clearMessages: () => void;
  setTyping: (typing: boolean) => void;
  setCurrentUser: (user: string) => void;
}

const useChatStore = create<ChatState>()(
  sync(
    (set, get) => ({
      messages: [],
      isTyping: false,
      currentUser: 'Anonymous',

      addMessage: (name, text) => {
        const newMessage: ChatMessage = {
          id: Date.now().toString() + Math.random().toString(36).substr(2, 9),
          name,
          text,
          timestamp: new Date(),
        };

        set(state => ({
          messages: [...state.messages, newMessage],
          isTyping: false,
        }));
      },

      clearMessages: () => {
        set({ messages: [] });
      },

      setTyping: typing => {
        set({ isTyping: typing });
      },

      setCurrentUser: user => {
        set({ currentUser: user });
      },
    }),
    { path: '/src/stores/chat-store.json' }
  )
);

export default useChatStore;
```

## Common Mistakes to Avoid

1. ❌ **DON'T** import create or sync - they're globally available
2. ❌ **DON'T** forget the sync wrapper
3. ❌ **DON'T** forget the path configuration
4. ❌ **DON'T** use named exports - always use default export
5. ❌ **DON'T** mutate state directly - always create new objects/arrays
6. ❌ **DON'T** forget the comment at the top of the file
7. ❌ **DON'T** use different formatting - the HMR system expects this exact pattern

## Validation Feedback

When creating stores, the Tonk system will validate your code and provide feedback:

- Syntax errors must be fixed before the file can be saved
- The system will auto-format your code to match style requirements
- Type errors will be reported and must be fixed
- Unused variables will cause validation failures

Remember: The HMR compilation system depends on this exact format. Deviating from it will break hot
reloading and state persistence.

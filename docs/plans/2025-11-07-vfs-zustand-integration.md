# VFS-Zustand Integration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan
> task-by-task.

**Goal:** Migrate berlin stores from localStorage to VFS persistence while preserving factory
pattern and immer middleware.

**Architecture:** Update StoreBuilder to support VFS sync configuration type. Apply middleware
initialization guard from demo. Migrate chatStore, presenceStore, and editorStore to VFS with
in-memory fallback.

**Tech Stack:** Zustand, VFS (Virtual File System), Immer, TypeScript

---

## Task 1: Fix middleware.ts initialization guard

**Files:**

- Modify: `examples/berlin/app/src/lib/middleware.ts:112-148`

**Context:** Berlin's middleware is missing the initialization guard that demo has. This can cause
multiple initializations when VFS reconnects.

**Step 1: Add initialization guard to initializeFromFile**

In `examples/berlin/app/src/lib/middleware.ts`, update the `initializeFromFile` function at line
112:

```typescript
// Initialize state from file if it exists
const initializeFromFile = async (state: T) => {
  // Guard against multiple initializations
  if (isInitialized) {
    console.log(`Already initialized for ${options.path}, skipping`);
    return;
  }

  console.log(`Checking if file exists: ${options.path}`);
  if (await vfs.exists(options.path)) {
    // ... rest of function unchanged
```

**Step 2: Verify the change**

Read the file to confirm the guard is in place:

Run: `cat examples/berlin/app/src/lib/middleware.ts | grep -A 5 "initializeFromFile"`

Expected: Should see the `if (isInitialized)` guard at the start of the function.

**Step 3: Commit**

```bash
git add examples/berlin/app/src/lib/middleware.ts
git commit -m "fix(berlin): add initialization guard to middleware

Prevents multiple VFS initializations when connection state changes.
Matches fix from demo middleware.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: Update StoreBuilder types

**Files:**

- Modify: `examples/berlin/app/src/lib/storeBuilder.ts:1-66`

**Context:** StoreBuilder currently only supports persist config. Need to add VFS config type and
type discrimination.

**Step 1: Update imports**

At the top of `examples/berlin/app/src/lib/storeBuilder.ts`:

```typescript
import { create } from 'zustand';
import type { PersistStorage } from 'zustand/middleware';
import { persist } from 'zustand/middleware';
import { immer } from 'zustand/middleware/immer';
import { sync } from './middleware';
```

**Step 2: Define config types**

Replace the `PersistConfig` type (lines 6-11) with:

```typescript
export type VFSConfig = {
  type: 'vfs';
  path: string;
};

export type PersistConfig = {
  type: 'persist';
  name: string;
  storage?: PersistStorage<unknown>;
  partialize?: <T>(state: T) => object;
  version?: number;
};

export type StoreConfig = VFSConfig | PersistConfig;
```

**Step 3: Update StoreBuilder signature**

Change the function signature at line 13:

```typescript
export const StoreBuilder = <T>(
	initialState: T,
	config: StoreConfig,
) => {
```

**Step 4: Add type discrimination logic**

Replace the store creation logic (lines 17-38) with:

```typescript
// Create store with VFS sync or persistence based on config type
const useStore =
  config.type === 'vfs'
    ? create<T>()(
        sync(
          immer(() => ({
            ...initialState,
          })),
          {
            path: config.path,
          }
        )
      )
    : create<T>()(
        persist(
          immer(() => ({
            ...initialState,
          })),
          {
            name: config.name,
            storage: config.storage,
            partialize: config.partialize as (state: T) => object,
            version: config.version,
          }
        )
      );
```

**Step 5: Verify TypeScript compiles**

Run: `cd examples/berlin/app && npm run typecheck`

Expected: Should compile without errors (existing stores will have type errors, we'll fix those
next).

**Step 6: Commit**

```bash
git add examples/berlin/app/src/lib/storeBuilder.ts
git commit -m "feat(berlin): add VFS config type to StoreBuilder

StoreBuilder now supports both VFS and persist configurations.
VFS stores use sync middleware, persist stores use persist middleware.
Both use immer for draft mutations.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: Migrate chatStore to VFS

**Files:**

- Modify: `examples/berlin/app/src/features/chat/stores/chatStore.ts:1-162`

**Context:** chatStore currently uses localStorage with createSafeStorage wrapper. Migrate to VFS
config.

**Step 1: Remove localStorage wrapper**

Delete the `createSafeStorage` function (lines 26-41):

```typescript
// DELETE THIS:
// Safe storage wrapper that handles unavailable localStorage
const createSafeStorage = (): StateStorage => {
  try {
    // Test if localStorage is available
    const test = '__test__';
    localStorage.setItem(test, test);
    localStorage.removeItem(test);
    return localStorage;
  } catch {
    // Return no-op storage if localStorage is unavailable
    return {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
    };
  }
};
```

**Step 2: Remove PersistStorage import**

At line 3, remove the import:

```typescript
// REMOVE: import type { PersistStorage, StateStorage } from 'zustand/middleware';
```

**Step 3: Update store configuration**

Replace lines 43-53 with:

```typescript
export const chatStore = StoreBuilder(initialState, {
  type: 'vfs',
  path: '/stores/chat.json',
});
```

**Step 4: Verify TypeScript compiles**

Run: `cd examples/berlin/app && npm run typecheck`

Expected: chatStore should compile without errors.

**Step 5: Commit**

```bash
git add examples/berlin/app/src/features/chat/stores/chatStore.ts
git commit -m "feat(berlin): migrate chatStore to VFS

Replace localStorage persistence with VFS sync.
Remove createSafeStorage wrapper (not needed with VFS).
Store persists to /stores/chat.json.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: Migrate presenceStore to VFS

**Files:**

- Modify: `examples/berlin/app/src/features/presence/stores/presenceStore.ts:1-162`

**Context:** presenceStore uses localStorage. Migrate to VFS while preserving cleanup timer logic.

**Step 1: Remove localStorage wrapper**

Delete the `createSafeStorage` function (lines 38-53):

```typescript
// DELETE THIS:
// Safe storage wrapper that handles unavailable localStorage
const createSafeStorage = (): StateStorage => {
  try {
    // Test if localStorage is available
    const test = '__test__';
    localStorage.setItem(test, test);
    localStorage.removeItem(test);
    return localStorage;
  } catch {
    // Return no-op storage if localStorage is unavailable
    return {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
    };
  }
};
```

**Step 2: Remove PersistStorage import**

At line 2, remove the import:

```typescript
// REMOVE: import type { PersistStorage, StateStorage } from 'zustand/middleware';
```

**Step 3: Update store configuration**

Replace lines 56-64 with:

```typescript
export const presenceStore = StoreBuilder(initialState, {
  type: 'vfs',
  path: '/stores/presence.json',
});
```

**Step 4: Verify TypeScript compiles**

Run: `cd examples/berlin/app && npm run typecheck`

Expected: presenceStore should compile without errors.

**Step 5: Commit**

```bash
git add examples/berlin/app/src/features/presence/stores/presenceStore.ts
git commit -m "feat(berlin): migrate presenceStore to VFS

Replace localStorage persistence with VFS sync.
Remove createSafeStorage wrapper.
Store persists to /stores/presence.json.
Cleanup timer logic unchanged.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 5: Migrate editorStore to VFS and StoreBuilder

**Files:**

- Modify: `examples/berlin/app/src/features/editor/stores/editorStore.ts:1-41`

**Context:** editorStore uses plain zustand create() with VFS sync commented out. Migrate to
StoreBuilder pattern and enable VFS.

**Step 1: Update imports**

Replace lines 1-3:

```typescript
import { StoreBuilder } from '../../../lib/storeBuilder';
import type { JSONContent } from '@tiptap/react';
```

**Step 2: Define initial state**

After the EditorState interface (line 14), add:

```typescript
const initialState: EditorState = {
  document: null,
  metadata: {
    title: 'Untitled',
  },
};
```

**Step 3: Replace store creation**

Replace lines 17-40 with:

```typescript
export const editorStore = StoreBuilder(initialState, {
  type: 'vfs',
  path: '/stores/editor.json',
});

export const useEditorStore = editorStore.useStore;

const createEditorActions = () => {
  const store = editorStore;

  return {
    setDocument: (doc: JSONContent) => {
      store.set(state => {
        state.document = doc;
      });
    },

    setTitle: (title: string) => {
      store.set(state => {
        state.metadata.title = title;
      });
    },

    setMetadata: (metadata: { title: string }) => {
      store.set(state => {
        state.metadata = metadata;
      });
    },

    clearDocument: () => {
      store.set(state => {
        state.document = null;
        state.metadata = { title: 'Untitled' };
      });
    },
  };
};

export const useEditor = editorStore.createFactory(createEditorActions());
```

**Step 4: Verify TypeScript compiles**

Run: `cd examples/berlin/app && npm run typecheck`

Expected: editorStore should compile without errors.

**Step 5: Commit**

```bash
git add examples/berlin/app/src/features/editor/stores/editorStore.ts
git commit -m "feat(berlin): migrate editorStore to VFS and StoreBuilder

Replace plain zustand with StoreBuilder pattern.
Enable VFS sync (was commented out due to connection issues).
Add factory pattern for consistency with other stores.
Store persists to /stores/editor.json.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 6: Verify all stores compile and run

**Files:**

- All store files

**Context:** Ensure the entire app compiles and all stores work correctly.

**Step 1: Run full typecheck**

Run: `cd examples/berlin/app && npm run typecheck`

Expected: No TypeScript errors.

**Step 2: Build the app**

Run: `cd examples/berlin/app && npm run build`

Expected: Build completes successfully.

**Step 3: Start dev server (manual testing)**

Run: `cd examples/berlin/app && npm run dev`

Expected: App starts without errors.

**Manual testing checklist:**

1. Open app in browser
2. Interact with chat (add messages)
3. Reload page - chat messages should persist
4. Check browser dev tools for VFS files:
   - `/stores/chat.json`
   - `/stores/presence.json`
   - `/stores/editor.json`
5. Open second tab - state should sync
6. Edit document in one tab - should sync to other tab

**Step 4: Commit verification**

If all tests pass, no commit needed. If issues found, fix and commit.

---

## Success Criteria

- [x] middleware.ts has initialization guard
- [x] StoreBuilder supports VFS config type
- [x] chatStore uses VFS at `/stores/chat.json`
- [x] presenceStore uses VFS at `/stores/presence.json`
- [x] editorStore uses VFS at `/stores/editor.json`
- [x] All stores use factory pattern
- [x] All stores use immer for mutations
- [x] TypeScript compiles without errors
- [x] App builds successfully
- [x] State persists across reloads
- [x] State syncs across tabs

## Optional Task 7: Migrate pixelStore to StoreBuilder

**Files:**

- Modify: `examples/berlin/app/src/stores/pixelStore.ts:1-54`

**Context:** pixelStore already uses VFS via raw sync middleware. Optionally migrate to StoreBuilder
for consistency.

**Step 1: Update imports**

Replace line 1-2:

```typescript
import { StoreBuilder } from '../lib/storeBuilder';
```

**Step 2: Define initial state**

After the PixelState interface:

```typescript
const initialState: PixelState = {
  pixels: {},
  selectedColor: '#000000',
};
```

**Step 3: Replace store creation**

Replace lines 17-53:

```typescript
export const pixelStore = StoreBuilder(initialState, {
  type: 'vfs',
  path: '/stores/pixels.json',
});

export const usePixelStore = pixelStore.useStore;

const createPixelActions = () => {
  const store = pixelStore;

  return {
    setPixel: (x: number, y: number, color: string) => {
      const key = `${x},${y}`;
      store.set(state => {
        state.pixels[key] = { color };
      });
    },

    removePixel: (x: number, y: number) => {
      const key = `${x},${y}`;
      store.set(state => {
        delete state.pixels[key];
      });
    },

    setSelectedColor: (color: string) => {
      store.set(state => {
        state.selectedColor = color;
      });
    },

    clearPixels: () => {
      store.set(state => {
        state.pixels = {};
      });
    },
  };
};

export const usePixel = pixelStore.createFactory(createPixelActions());
```

**Step 4: Update components using pixelStore**

Find components importing `usePixelStore` and update to use `usePixel` if they need actions.

**Step 5: Verify and commit**

Run: `cd examples/berlin/app && npm run typecheck && npm run build`

```bash
git add examples/berlin/app/src/stores/pixelStore.ts
git commit -m "refactor(berlin): migrate pixelStore to StoreBuilder

Adopt StoreBuilder pattern for consistency.
Add factory pattern with actions.
Maintains VFS sync at /stores/pixels.json.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com)"
```

---

## Notes

- The middleware fix (Task 1) prevents race conditions when VFS reconnects
- VFS fallback to in-memory is automatic when VFS unavailable
- No localStorage fallback needed - middleware handles graceful degradation
- Factory pattern preserved in all stores for action encapsulation
- Immer middleware enables draft mutations (state.property = value)

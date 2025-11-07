# VFS-Zustand Integration Design

**Date:** 2025-11-07 **Status:** Approved **Branch:** ramram/feat/berlin

## Problem

The berlin example has three stores (chatStore, presenceStore, editorStore) that persist to
localStorage. The demo example shows how to sync stores with VFS using the `sync` middleware. We
need to integrate VFS sync into berlin's StoreBuilder pattern while preserving the factory pattern
and immer middleware.

## Solution

Migrate all berlin stores to VFS persistence with in-memory fallback. Update StoreBuilder to support
both VFS and localStorage configurations. Apply middleware fixes from demo to prevent initialization
race conditions.

## Architecture

### Store Configuration

StoreBuilder will accept two config types:

```typescript
type VFSConfig = {
  type: 'vfs';
  path: string;
};

type PersistConfig = {
  type: 'persist';
  name: string;
  storage?: PersistStorage<unknown>;
  partialize?: <T>(state: T) => object;
  version?: number;
};
```

All berlin stores will use VFS config:

```typescript
const chatStore = StoreBuilder(initialState, {
  type: 'vfs',
  path: '/stores/chat.json',
});
```

### Middleware Composition

**VFS stores:** `create()(sync(immer(stateCreator), { path }))` **Persist stores:**
`create()(persist(immer(stateCreator), { name, storage, ... }))`

Immer wraps the state creator. Sync or persist wraps immer. This preserves draft mutation in all
stores.

### Fallback Behavior

When VFS disconnects, stores operate in-memory. No localStorage fallback. No operation queuing. The
middleware already handles this.

## Implementation

### 1. Fix middleware.ts

Apply the initialization guard from demo (missing in berlin):

```typescript
const initializeFromFile = async (state: T) => {
  if (isInitialized) {
    console.log(`Already initialized for ${options.path}, skipping`);
    return;
  }
  // ... rest of initialization
};
```

Do not restore the `waitForVFS()` polling function. The connection state listener is sufficient.

### 2. Update StoreBuilder

Add type discrimination logic:

```typescript
export const StoreBuilder = <T>(initialState: T, config: VFSConfig | PersistConfig) => {
  const useStore =
    config.type === 'vfs'
      ? create<T>()(
          sync(
            immer(() => initialState),
            { path: config.path }
          )
        )
      : create<T>()(
          persist(
            immer(() => initialState),
            {
              name: config.name,
              storage: config.storage,
              partialize: config.partialize,
              version: config.version,
            }
          )
        );

  // ... rest unchanged
};
```

### 3. Migrate Stores

**chatStore:**

- Change config to `{ type: 'vfs', path: '/stores/chat.json' }`
- Remove `createSafeStorage()` wrapper
- Remove `partialize` option
- Keep factory pattern unchanged

**presenceStore:**

- Change config to `{ type: 'vfs', path: '/stores/presence.json' }`
- Remove `createSafeStorage()` wrapper
- Keep cleanup interval logic

**editorStore:**

- Migrate from plain `create()` to `StoreBuilder`
- Use config `{ type: 'vfs', path: '/stores/editor.json' }`
- Remove TEMP comments about connection issues
- Add factory pattern for consistency

**pixelStore:**

- Already uses VFS via sync middleware
- Optionally migrate to StoreBuilder for consistency

## Testing

Test each store manually:

1. Verify state persists across page reloads
2. Open multiple tabs, verify state syncs
3. Disconnect VFS (stop service worker), verify in-memory operation
4. Reconnect VFS, verify state loads
5. Verify factory pattern still works
6. Verify immer mutations still work

## Success Criteria

- All stores write to VFS files
- State survives page reloads
- Multiple tabs stay synchronized
- No errors when VFS unavailable
- Factory pattern preserved
- Immer mutations preserved
- editorStore connection issues resolved

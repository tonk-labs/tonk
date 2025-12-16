# Gotchas and Known Issues

## Critical Gotchas

### VFS Connection Required

The app blocks rendering until VFS connects. See `main.tsx:13-21`.

```typescript
// Wrong - will fail
const data = await vfs.readFile('/path');

// Right - wait for connection
await vfs.connect();
const data = await vfs.readFile('/path');
```

### 2-Second Save Debounce

Sync middleware debounces VFS writes by 2 seconds. Rapid changes vanish if the app closes unexpectedly.

**Mitigation:** Editor flushes pending saves on unmount.

**Location:** `src/lib/middleware.ts:64`

### Never Hardcode VFS Paths

Use FHS constants from `src/lib/paths.ts`:

```typescript
// Wrong
const path = '/var/lib/desktonk/layout/myfile.json';

// Right
import { FHS } from '@lib/paths';
const path = FHS.getLayoutPath('myfile');
```

### Binary vs Text File Reading

VFS files can store content in two ways:
- `content.text` - JSON-serializable text
- `bytes` - Base64-encoded binary data

Always check both:

```typescript
const text = file.content?.text ?? vfs.readBytesAsString(file.bytes);
```

### Canvas Persistence Race Condition

Wait for `canvasPersistenceReady` before initializing DesktopService. Shapes created too early cause data loss.

**Location:** `src/features/desktop/components/Desktop.tsx:290-300`

### Thumbnail Cache Invalidation

Call `invalidateThumbnailCache()` after regenerating thumbnails. Components otherwise display stale images.

```typescript
import { invalidateThumbnailCache } from '../hooks/useThumbnail';

await generateNewThumbnail(path);
invalidateThumbnailCache(path);
```

### currentUserId Protection

The presence store protects `currentUserId` from VFS sync overwrites via a subscriber pattern. Stores with local-only fields need similar protection.

**Location:** `src/features/presence/store.ts`

---

## Development Gotchas

### mprocs Cleans CRDT Data

Dev environment deletes `automerge-repo-data` on each start. Expect state loss between sessions.

### Strict Port 4001

Dev server runs on port 4001. Changing it requires updating both `vite.config.ts` and launcher config.

### rolldown-vite Override

This package uses `rolldown-vite`, not standard Vite (`package.json:76`). Some edge cases behave differently.

### Run Full Environment

```bash
# Wrong - missing launcher and relay
bunx vite

# Right - starts all three processes
bun run dev
```

---

## TLDraw Gotchas

### Shape ID Convention

Shape IDs must follow `shape:{type}:{fileId}`:

```typescript
// Wrong
const id = createShapeId();

// Right
const id = createShapeId(`file-icon:${fileId}`);
```

### Theme in onMount, Not useEffect

Set TLDraw theme in `onMount` callback, not useEffect, to prevent flash.

### track() Wrapper Required

Components reading TLDraw editor state need `track()` for re-renders. See [tldraw docs](https://tldraw.dev/docs/editor#primitives).

### Icon CSS Masks

TLDraw icons use CSS masks. Without `background-color: currentColor`, icons appear invisible.

**Fixed in:** `src/global.css`

---

## State Management Gotchas

### partialize Fields

Include only collaborative fields in `partialize`. VFS sync overwrites local-only fields.

```typescript
StoreBuilder(initialState, undefined, {
  path: '...',
  partialize: (state) => ({
    syncedField: state.syncedField,
    // Do NOT include localOnlyField
  }),
});
```

### localStorage Fields in VFS Store

VFS-synced stores with localStorage fields need a subscriber to restore them after sync:

```typescript
store.subscribe((state, prevState) => {
  if (state.localField !== prevState.localField) {
    localStorage.setItem('localField', state.localField);
  }
});
```

### Feature Flag Migrations

Increment version and add migration logic when adding flags:

```typescript
StoreBuilder(initialState, {
  name: 'feature-flags',
  version: 2, // Bumped from 1
  migrate: (persisted, version) => {
    if (version < 2) {
      return { ...persisted, newFlag: false };
    }
    return persisted;
  },
});
```

---

## Debounce Timings

Different subsystems use different debounce values:

| System | Delay | Purpose |
|--------|-------|---------|
| Sync middleware | 2000ms | VFS state persistence |
| Editor auto-save | 1000ms | File content save |
| Position save | 500ms | Icon position to VFS |
| File change handling | 100ms | DesktopService coalescing |

---

## Known Limitations

### EditorStore VFS Sync Disabled

Editor store uses plain Zustand without VFS sync. Code marks this "TEMP" due to connection issues.

**Location:** `src/features/editor/store.ts`

### Service Worker Lives in Launcher

The Service Worker lives in the launcher package. Investigating VFS internals requires that package.

### Production vs Development Differ

Some behaviors change between `bun run dev` and production bundles. Test production before delivery.

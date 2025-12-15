# Getting Started

## Prerequisites

This monorepo uses **Bun**. Avoid npm, yarn, and pnpm.

```bash
# From monorepo root
bun install
```

## Development

```bash
# From packages/desktonk
bun run dev
```

mprocs starts three processes:
- Launcher (parent window)
- Desktonk (this app)
- Relay (sync server)

**Run all three.** Vite alone fails because the app needs the launcher and relay.

## Testing with Sample Files

In the browser console:

```javascript
createSampleFiles()
```

This creates `Welcome.txt`, `README.md`, and `notes.json`.

## Building

```bash
bun run build    # Vite build to dist/
bun run bundle   # Build + create app.tonk
```

## Adding a New Feature

### 1. Create Feature Directory

```mermaid
graph TB
    subgraph "src/features/my-feature"
        index["index.ts<br/><i>Public exports</i>"]
        components["components/<br/><i>React components</i>"]
        hooks["hooks/<br/><i>Custom hooks</i>"]
        stores["stores/<br/><i>Zustand stores (optional)</i>"]
        types["types.ts<br/><i>TypeScript types</i>"]
        utils["utils/<br/><i>Helper functions</i>"]
    end
```

### 2. Choose State Management Strategy

`StoreBuilder` creates Zustand stores with your chosen persistence:

```typescript
import { StoreBuilder } from '@lib/storeBuilder';
import { FHS } from '@lib/paths';

// VFS sync - collaborative state
const vfsSyncedStore = StoreBuilder(initialState, undefined, {
  path: FHS.getServicePath('my-feature', 'state.json'),
  partialize: (state) => ({ syncedField: state.syncedField }),
});

// localStorage - local preferences
const localStore = StoreBuilder(initialState, {
  name: 'my-feature',
  version: 1,
  storage: localStorage,
});

// No persistence - session only
const sessionStore = StoreBuilder(initialState);
```

### 3. Create Store with Actions

```typescript
interface MyState {
  data: string;
  localOnly: boolean;
}

const initialState: MyState = { data: '', localOnly: false };

export const myStore = StoreBuilder(initialState, undefined, {
  path: FHS.getServicePath('my-feature', 'state.json'),
  partialize: (state) => ({ data: state.data }), // Only sync 'data'
});

const createMyActions = () => ({
  setData: (value: string) => {
    myStore.set((state) => {
      state.data = value;  // Immer-style mutation
    });
  },
});

export const useMyFeature = myStore.createFactory(createMyActions());
```

### 4. Add Route (if needed)

Edit `src/Router.tsx`:

```typescript
import { MyFeature } from './features/my-feature';

// Inside Routes
<Route path="/my-feature" element={<MyFeature />} />
```

## Adding a Custom TLDraw Shape

### 1. Create Shape Util

```typescript
// src/features/desktop/shapes/MyShapeUtil.tsx
import { ShapeUtil, T, Rectangle2d, HTMLContainer } from 'tldraw';

interface MyShape {
  type: 'my-shape';
  props: {
    label: string;
    optional: string | null;
  };
}

export class MyShapeUtil extends ShapeUtil<MyShape> {
  static override type = 'my-shape' as const;

  static override props = {
    label: T.string,
    optional: T.string.nullable().optional(),  // Use this pattern!
  };

  getDefaultProps(): MyShape['props'] {
    return { label: 'Default', optional: null };
  }

  getGeometry(shape: MyShape) {
    return new Rectangle2d({ width: 100, height: 100, isFilled: true });
  }

  component(shape: MyShape) {
    return (
      <HTMLContainer>
        <div>{shape.props.label}</div>
      </HTMLContainer>
    );
  }

  indicator(shape: MyShape) {
    return <rect width={100} height={100} />;
  }
}
```

### 2. Register Shape

Edit `src/features/desktop/components/Desktop.tsx`:

```typescript
import { MyShapeUtil } from '../shapes/MyShapeUtil';

const customShapeUtils = [FileIconUtil, MyShapeUtil];
```

## Working with VFS

### Reading Files

```typescript
import { getVFSService } from '@/vfs-client';

const vfs = getVFSService();
await vfs.connect();

const file = await vfs.readFile('/desktonk/myfile.txt');

// Text files store data in content or bytes—check both
const text = file.content?.text ?? vfs.readBytesAsString(file.bytes);
```

### Writing Files

```typescript
// Text content
await vfs.writeFile('/desktonk/myfile.txt', {
  content: { text: 'Hello', desktopMeta: { ... } },
});

// Binary content
await vfs.writeFile('/desktonk/image.png', {
  bytes: base64String,
  content: { desktopMeta: { ... } },
});
```

### Watching for Changes

```typescript
const unwatch = await vfs.watch('/desktonk/', (event) => {
  console.log('File changed:', event.path);
});

unwatch(); // Stop watching
```

## Common Tasks

### Access Feature Flags

```typescript
import { useFeatureFlags } from '@lib/featureFlags';

function MyComponent() {
  const { plainTextMode, togglePlainTextMode } = useFeatureFlags();
  // ...
}
```

### Use Theme Hook

```typescript
import { useTheme } from '@/hooks/useTheme';

function MyComponent() {
  const { isDark, toggle } = useTheme();
  // ...
}
```

### Invalidate Thumbnail Cache

```typescript
import { invalidateThumbnailCache } from '../hooks/useThumbnail';

// After regenerating a thumbnail
invalidateThumbnailCache('/var/lib/desktonk/thumbnails/myfile.png');
```

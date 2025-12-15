# Getting Started Guide

This guide shows how to add new features to the Tonk Launcher.

## Prerequisites

- Bun installed (`brew install oven-sh/bun/bun`)
- Chrome or Safari (Firefox lacks support)
- Familiarity with React and TypeScript

## Project Structure

```
src/
├── App.tsx                    # Launcher main component
├── main.tsx                   # Launcher entry point
├── launcher/                  # Launcher-specific code
│   ├── services/              # Business logic (bundleStorage, bundleManager)
│   └── sw/                    # Service worker (separate build target)
├── runtime/                   # Runtime app (iframe, separate build)
│   ├── RuntimeApp.tsx         # Runtime main component
│   └── hooks/                 # Runtime-specific hooks
├── components/                # Shared UI components
├── hooks/                     # Shared hooks
└── lib/                       # Utilities (cn for classnames)
```

## Adding a New Component

1. Create a folder under `src/components/`:

```
src/components/myButton/
└── myButton.tsx
```

2. Write the component:

```typescript
import { cn } from '@/lib/utils';

export interface MyButtonProps {
  className?: string;
  label: string;
  onClick?: () => void;
}

export function MyButton({ className, label, onClick }: MyButtonProps) {
  return (
    <button
      className={cn("bg-stone-100 dark:bg-night-800 p-2 rounded", className)}
      onClick={onClick}
    >
      {label}
    </button>
  );
}
```

3. Import using the path alias:

```typescript
import { MyButton } from '@/components/myButton/myButton';
```

**Rules enforced by ast-grep:**
- Use `cn()` for className composition
- No cross-imports between `launcher/` and `runtime/`

## Adding a New Hook

Create in `src/hooks/` for shared hooks or `src/runtime/hooks/` for runtime-specific hooks.

```typescript
// src/hooks/useMyFeature.ts
import { useState, useCallback } from 'react';

export function useMyFeature() {
  const [data, setData] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    // implementation
  }, []);

  return { data, refresh };
}
```

**Rules:**
- Hook files must start with `use`
- Hook functions must start with `use`

## Adding a New Service

Create in `src/launcher/services/`:

```typescript
// src/launcher/services/myService.ts
export class MyService {
  private initialized = false;

  async initialize(): Promise<void> {
    if (this.initialized) return;
    // setup logic
    this.initialized = true;
  }

  async doOperation(param: string): Promise<Result> {
    await this.initialize();
    // operation logic
  }
}

// Export singleton
export const myService = new MyService();
```

Add tests in `src/launcher/services/__tests__/myService.test.ts`.

## Adding a Service Worker Message Handler

The service worker communicates with the runtime via messages. To add a new operation:

### Step 1: Define message types

Edit `src/launcher/types.ts`:

```typescript
export type VFSWorkerMessage =
  // ... existing types
  | { type: 'myOperation'; id: string; param: string };

export type VFSWorkerResponse =
  // ... existing types
  | { type: 'myOperation'; id: string; success: boolean; data?: Result; error?: string };
```

### Step 2: Create the handler

Create `src/launcher/sw/message-handlers/my-ops.ts`:

```typescript
import { getActiveBundle } from '../state';
import { logger } from '../utils/logging';
import { postResponse } from '../utils/response';

export async function handleMyOperation({ id, param }: { id: string; param: string }) {
  const activeBundle = getActiveBundle();

  if (!activeBundle) {
    await postResponse({ type: 'myOperation', id, success: false, error: 'Bundle not active' });
    return;
  }

  try {
    const result = await activeBundle.tonk.someMethod(param);
    await postResponse({ type: 'myOperation', id, success: true, data: result });
  } catch (error) {
    logger.error('myOperation failed', { error });
    await postResponse({
      type: 'myOperation',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error)
    });
  }
}
```

### Step 3: Register the handler

Edit `src/launcher/sw/message-handlers/index.ts`:

```typescript
import { handleMyOperation } from './my-ops';

// In the switch statement:
case 'myOperation':
  await handleMyOperation({ id: msgId, param: message.param as string });
  break;
```

### Step 4: Rebuild the service worker

```bash
vite build -c vite.sw.config.ts && cp dist-sw/service-worker-bundled.js public/app/
```

**Important:** If the operation should work before a bundle loads, add `'myOperation'` to the `allowedWhenUninitialized` array.

## Adding a Runtime Screen

Runtime screens display during the bundle loading lifecycle (loading, error, etc.).

### Step 1: Add screen state

Edit `src/runtime/types.ts` (or wherever screen states are defined):

```typescript
export enum ScreenState {
  LOADING = 'loading',
  ERROR = 'error',
  MY_SCREEN = 'my_screen',
}
```

### Step 2: Create the screen component

Create `src/runtime/components/screens/MyScreen.tsx`:

```typescript
export function MyScreen() {
  return (
    <div className="flex items-center justify-center h-full">
      <p>My custom screen</p>
    </div>
  );
}
```

### Step 3: Add to RuntimeApp

Edit `src/runtime/RuntimeApp.tsx`:

```typescript
import { MyScreen } from './components/screens/MyScreen';

// In the render:
{screenState === ScreenState.MY_SCREEN && <MyScreen />}
```

### Step 4: Rebuild the runtime

```bash
vite build -c vite.runtime.config.ts && cp -r dist-runtime/* public/app/
```

## Build System

The project uses three Vite configurations:

- `vite.config.ts` — Launcher app to `dist/`
- `vite.runtime.config.ts` — Runtime app to `dist-runtime/` then `public/app/`
- `vite.sw.config.ts` — Service worker to `dist-sw/` then `public/app/`

### Development Workflow

```bash
# Start everything (launcher + SW watching)
bun run dev

# Rebuild runtime after changes
vite build -c vite.runtime.config.ts && cp -r dist-runtime/* public/app/

# Rebuild SW after changes
vite build -c vite.sw.config.ts && cp dist-sw/service-worker-bundled.js public/app/
```

### Production Build

```bash
bun run build:runtime  # Builds both runtime and SW, copies to public/app/
bun run build          # Builds launcher
```

## Testing

```bash
# Run all tests
bun test

# Watch mode
bun run test:watch

# With coverage
bun run test:coverage
```

Tests use Vitest with `fake-indexeddb` for IndexedDB mocking.

## Linting

```bash
# Run all lints
bun run lint

# Fix auto-fixable issues
bun run lint:fix

# Format code
bun run format
```

The project enforces architectural rules via ast-grep:
- No React imports in service worker code
- No cross-imports between launcher and runtime
- Hook naming conventions
- Use `cn()` for className composition

## Next Steps

- Read [Gotchas](./gotchas.md) to avoid common pitfalls
- Review [Architecture](./architecture.md) for the complete technical reference
- Check `.ast-grep/rules/` to understand enforced patterns

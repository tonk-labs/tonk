# Presence Feature Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan
> task-by-task.

**Goal:** Build Google Docs-style presence indicators showing active users with auto-assigned animal
names and colors.

**Architecture:** StoreBuilder pattern with immer/persist middleware, deterministic user generation
from browser fingerprint, centralized 30s cleanup timer, throttled activity tracking via custom
hook.

**Tech Stack:** Zustand, StoreBuilder, unique-names-generator, lodash.throttle, Tailwind CSS, React
hooks

---

## Task 1: Setup Dependencies and StoreBuilder

**Files:**

- Copy: `/Users/alexander/Node/storebuilder.ts` → `app/src/lib/storeBuilder.ts`
- Create: `app/package.json` (modify dependencies)

**Step 1: Copy StoreBuilder to project**

```bash
cp /Users/alexander/Node/storebuilder.ts app/src/lib/storeBuilder.ts
```

**Step 2: Install unique-names-generator dependency**

```bash
cd app && bun add unique-names-generator
```

Expected: Package installed successfully

**Step 3: Verify StoreBuilder compiles**

```bash
cd app && bun run build
```

Expected: No TypeScript errors related to storeBuilder.ts

**Step 4: Commit**

```bash
git add app/src/lib/storeBuilder.ts app/package.json app/bun.lock
git commit -m "feat(presence): add StoreBuilder and dependencies"
```

---

## Task 2: Color Palette Constant

**Files:**

- Create: `app/src/lib/constants/presenceColors.ts`

**Step 1: Create color palette constant**

```typescript
export const PRESENCE_COLORS = [
  '#FF6B6B', // Coral Red
  '#4ECDC4', // Turquoise
  '#45B7D1', // Sky Blue
  '#FFA07A', // Light Salmon
  '#98D8C8', // Mint Green
  '#F7DC6F', // Pastel Yellow
  '#BB8FCE', // Lavender
  '#85C1E2', // Light Blue
] as const;

export type PresenceColor = (typeof PRESENCE_COLORS)[number];
```

**Step 2: Verify it compiles**

```bash
cd app && bun run build
```

Expected: No TypeScript errors

**Step 3: Commit**

```bash
git add app/src/lib/constants/presenceColors.ts
git commit -m "feat(presence): add color palette constant"
```

---

## Task 3: User Generation Utils

**Files:**

- Create: `app/src/features/presence/utils/userGeneration.ts`

**Step 1: Create user generation utilities**

```typescript
import { uniqueNamesGenerator, adjectives, animals } from 'unique-names-generator';
import { PRESENCE_COLORS } from '../../../lib/constants/presenceColors';

/**
 * Generate deterministic color from userId using hash
 */
export const getUserColor = (userId: string): string => {
  const hash = userId.split('').reduce((acc, char) => char.charCodeAt(0) + ((acc << 5) - acc), 0);
  return PRESENCE_COLORS[Math.abs(hash) % PRESENCE_COLORS.length];
};

/**
 * Generate deterministic animal name from userId
 */
export const getUserName = (userId: string): string => {
  return uniqueNamesGenerator({
    dictionaries: [adjectives, animals],
    seed: userId,
    separator: ' ',
    style: 'capital',
  });
};

/**
 * Get initials from animal name (first letter of each word)
 */
export const getInitials = (name: string): string => {
  return name
    .split(' ')
    .map(word => word[0])
    .join('')
    .toUpperCase();
};

/**
 * Get or create persistent user ID in localStorage
 */
export const initializeUserId = (): string => {
  const STORAGE_KEY = 'tonk-user-id';
  let userId = localStorage.getItem(STORAGE_KEY);

  if (!userId) {
    userId = crypto.randomUUID();
    localStorage.setItem(STORAGE_KEY, userId);
  }

  return userId;
};
```

**Step 2: Verify it compiles**

```bash
cd app && bun run build
```

Expected: No TypeScript errors

**Step 3: Commit**

```bash
git add app/src/features/presence/utils/userGeneration.ts
git commit -m "feat(presence): add user generation utilities"
```

---

## Task 4: Presence Store (Core State)

**Files:**

- Create: `app/src/features/presence/stores/presenceStore.ts`

**Step 1: Create presence store with StoreBuilder**

```typescript
import { StoreBuilder } from '../../../lib/storeBuilder';
import { initializeUserId, getUserColor, getUserName } from '../utils/userGeneration';

export interface User {
  id: string;
  name: string;
  color: string;
  lastSeen: number;
}

interface PresenceState {
  users: Record<string, User>;
  currentUserId: string | null;
}

// Initialize current user
const userId = initializeUserId();
const initialUser: User = {
  id: userId,
  name: getUserName(userId),
  color: getUserColor(userId),
  lastSeen: Date.now(),
};

const initialState: PresenceState = {
  users: {
    [userId]: initialUser,
  },
  currentUserId: userId,
};

// Create store with localStorage persistence
export const presenceStore = StoreBuilder(initialState, {
  name: 'tonk-presence',
  version: 1,
});

// Export base hooks
export const usePresenceStore = presenceStore.useStore;
```

**Step 2: Verify it compiles**

```bash
cd app && bun run build
```

Expected: No TypeScript errors

**Step 3: Commit**

```bash
git add app/src/features/presence/stores/presenceStore.ts
git commit -m "feat(presence): add presence store with StoreBuilder"
```

---

## Task 5: Presence Store Actions

**Files:**

- Modify: `app/src/features/presence/stores/presenceStore.ts` (add actions)

**Step 1: Add actions factory to presenceStore.ts**

Add after `usePresenceStore` export:

```typescript
const FIVE_MINUTES_MS = 5 * 60 * 1000;

const createPresenceActions = () => {
  const store = presenceStore;

  return {
    /**
     * Update current user's lastSeen timestamp
     */
    updatePresence: () => {
      const { currentUserId } = store.get();
      if (!currentUserId) return;

      store.set(state => {
        if (state.users[currentUserId]) {
          state.users[currentUserId].lastSeen = Date.now();
        }
      });
    },

    /**
     * Remove users who haven't been active in last 5 minutes
     */
    pruneStaleUsers: () => {
      const cutoffTime = Date.now() - FIVE_MINUTES_MS;

      store.set(state => {
        Object.keys(state.users).forEach(id => {
          if (state.users[id].lastSeen < cutoffTime) {
            delete state.users[id];
          }
        });
      });
    },

    /**
     * Get all users active in last 5 minutes, sorted by lastSeen desc
     */
    getActiveUsers: (): User[] => {
      const { users } = store.get();
      const cutoffTime = Date.now() - FIVE_MINUTES_MS;

      return Object.values(users)
        .filter(user => user.lastSeen >= cutoffTime)
        .sort((a, b) => b.lastSeen - a.lastSeen);
    },
  };
};

// Export factory hook
export const usePresence = presenceStore.createFactory(createPresenceActions());
```

**Step 2: Verify it compiles**

```bash
cd app && bun run build
```

Expected: No TypeScript errors

**Step 3: Commit**

```bash
git add app/src/features/presence/stores/presenceStore.ts
git commit -m "feat(presence): add store actions for update/prune/getActive"
```

---

## Task 6: Cleanup Timer Management

**Files:**

- Modify: `app/src/features/presence/stores/presenceStore.ts` (add timer)

**Step 1: Add cleanup timer management**

Add after imports at top of file:

```typescript
// Cleanup timer management
let cleanupInterval: NodeJS.Timeout | null = null;
let subscriberCount = 0;

const CLEANUP_INTERVAL_MS = 30 * 1000; // 30 seconds

const startCleanupTimer = () => {
  if (cleanupInterval) return; // Already running

  cleanupInterval = setInterval(() => {
    const actions = createPresenceActions();
    actions.pruneStaleUsers();
  }, CLEANUP_INTERVAL_MS);
};

const stopCleanupTimer = () => {
  if (cleanupInterval) {
    clearInterval(cleanupInterval);
    cleanupInterval = null;
  }
};

// Track subscribers to manage timer lifecycle
presenceStore.subscribe(state => {
  // This runs on every state change, but we only care about subscriber count
  return state;
});
```

**Step 2: Add public timer control functions**

Add to exports at bottom:

```typescript
/**
 * Start cleanup timer (called by components on mount)
 */
export const startPresenceCleanup = () => {
  subscriberCount++;
  if (subscriberCount === 1) {
    startCleanupTimer();
  }
};

/**
 * Stop cleanup timer (called by components on unmount)
 */
export const stopPresenceCleanup = () => {
  subscriberCount--;
  if (subscriberCount === 0) {
    stopCleanupTimer();
  }
};
```

**Step 3: Verify it compiles**

```bash
cd app && bun run build
```

Expected: No TypeScript errors

**Step 4: Commit**

```bash
git add app/src/features/presence/stores/presenceStore.ts
git commit -m "feat(presence): add cleanup timer management"
```

---

## Task 7: Activity Tracking Hook

**Files:**

- Create: `app/src/features/presence/hooks/usePresenceTracking.ts`

**Step 1: Create activity tracking hook**

```typescript
import { useEffect } from 'react';
import throttle from 'lodash.throttle';
import { usePresence } from '../stores/presenceStore';

const THROTTLE_MS = 1000; // Max 1 update per second

/**
 * Track user activity and update presence
 * Monitors: mousemove, keydown on document
 */
export const usePresenceTracking = () => {
  const { updatePresence } = usePresence();

  useEffect(() => {
    // Throttled update function
    const throttledUpdate = throttle(
      () => {
        updatePresence();
      },
      THROTTLE_MS,
      { leading: true, trailing: false }
    );

    // Event listeners
    const handleActivity = () => {
      throttledUpdate();
    };

    // Attach listeners
    document.addEventListener('mousemove', handleActivity);
    document.addEventListener('keydown', handleActivity);

    // Cleanup
    return () => {
      document.removeEventListener('mousemove', handleActivity);
      document.removeEventListener('keydown', handleActivity);
      throttledUpdate.cancel();
    };
  }, [updatePresence]);
};
```

**Step 2: Verify it compiles**

```bash
cd app && bun run build
```

Expected: No TypeScript errors

**Step 3: Commit**

```bash
git add app/src/features/presence/hooks/usePresenceTracking.ts
git commit -m "feat(presence): add activity tracking hook"
```

---

## Task 8: PresenceIndicators Component

**Files:**

- Create: `app/src/features/presence/components/PresenceIndicators.tsx`

**Step 1: Create PresenceIndicators component**

```typescript
import { useEffect } from 'react';
import { usePresence, startPresenceCleanup, stopPresenceCleanup } from '../stores/presenceStore';
import { getInitials } from '../utils/userGeneration';

interface PresenceIndicatorsProps {
  className?: string;
  maxVisible?: number;
}

export const PresenceIndicators = ({
  className = '',
  maxVisible
}: PresenceIndicatorsProps) => {
  const { getActiveUsers } = usePresence();
  const activeUsers = getActiveUsers();

  // Manage cleanup timer lifecycle
  useEffect(() => {
    startPresenceCleanup();
    return () => {
      stopPresenceCleanup();
    };
  }, []);

  // Apply maxVisible limit
  const visibleUsers = maxVisible
    ? activeUsers.slice(0, maxVisible)
    : activeUsers;
  const hiddenCount = maxVisible && activeUsers.length > maxVisible
    ? activeUsers.length - maxVisible
    : 0;

  if (activeUsers.length === 0) {
    return null;
  }

  return (
    <div className={`flex items-center ${className}`}>
      {visibleUsers.map((user, index) => (
        <div
          key={user.id}
          className="relative group"
          style={{ marginLeft: index > 0 ? '-8px' : '0' }}
        >
          <div
            className="w-8 h-8 rounded-full flex items-center justify-center text-white text-sm font-medium border-2 border-white shadow-sm cursor-default"
            style={{ backgroundColor: user.color }}
            title={user.name}
          >
            {getInitials(user.name)}
          </div>

          {/* Tooltip */}
          <div className="absolute bottom-full left-1/2 transform -translate-x-1/2 mb-2 px-2 py-1 bg-gray-900 text-white text-xs rounded whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
            {user.name}
          </div>
        </div>
      ))}

      {hiddenCount > 0 && (
        <div className="w-8 h-8 rounded-full flex items-center justify-center bg-gray-300 text-gray-700 text-xs font-medium border-2 border-white shadow-sm ml-[-8px]">
          +{hiddenCount}
        </div>
      )}
    </div>
  );
};
```

**Step 2: Verify it compiles**

```bash
cd app && bun run build
```

Expected: No TypeScript errors

**Step 3: Commit**

```bash
git add app/src/features/presence/components/PresenceIndicators.tsx
git commit -m "feat(presence): add PresenceIndicators component"
```

---

## Task 9: Feature Index and Exports

**Files:**

- Create: `app/src/features/presence/index.ts`

**Step 1: Create feature barrel export**

```typescript
// Components
export { PresenceIndicators } from './components/PresenceIndicators';

// Hooks
export { usePresenceTracking } from './hooks/usePresenceTracking';

// Store
export {
  usePresenceStore,
  usePresence,
  startPresenceCleanup,
  stopPresenceCleanup,
} from './stores/presenceStore';

// Types
export type { User } from './stores/presenceStore';
```

**Step 2: Verify it compiles**

```bash
cd app && bun run build
```

Expected: No TypeScript errors

**Step 3: Commit**

```bash
git add app/src/features/presence/index.ts
git commit -m "feat(presence): add feature exports"
```

---

## Task 10: Integration Example in App

**Files:**

- Modify: `app/src/App.tsx`

**Step 1: Add presence to App.tsx**

Read current App.tsx:

```bash
cat app/src/App.tsx
```

**Step 2: Import and use presence components**

Add at top:

```typescript
import { PresenceIndicators, usePresenceTracking } from './features/presence';
```

Inside main App component function:

```typescript
// Enable presence tracking
usePresenceTracking();
```

Add component in render (suggested location - top-right corner):

```typescript
<PresenceIndicators className="fixed top-4 right-4 z-50" maxVisible={5} />
```

**Step 3: Verify it compiles**

```bash
cd app && bun run build
```

Expected: No TypeScript errors

**Step 4: Run dev server and test**

```bash
cd app && bun run dev
```

Open browser to http://localhost:4000 (or whatever port)

Expected:

- See your own presence circle appear
- Hover shows your animal name
- Mouse/keyboard activity updates timestamp
- Opening multiple tabs shows multiple users

**Step 5: Commit**

```bash
git add app/src/App.tsx
git commit -m "feat(presence): integrate presence indicators into App"
```

---

## Verification Checklist

- [ ] Color palette has 8 distinct colors
- [ ] User gets deterministic color (same user = same color across sessions)
- [ ] User gets deterministic animal name (same user = same name)
- [ ] Presence circle shows correct initials (e.g., "EP" for "Eager Panda")
- [ ] Hover tooltip displays full animal name
- [ ] Activity (mouse/keyboard) updates lastSeen
- [ ] Users inactive >5min disappear from list
- [ ] Multiple browser tabs create multiple distinct users
- [ ] Cleanup timer runs every 30 seconds
- [ ] localStorage persists user ID across page reloads
- [ ] Component can be placed anywhere with className prop
- [ ] maxVisible prop limits displayed users and shows "+N" badge

---

## Next Steps

After implementation and verification:

1. **Manual Testing:** Open multiple browser tabs/windows to simulate multiple users
2. **Test Cleanup:** Wait 5+ minutes with no activity to verify user removal
3. **Test Persistence:** Reload page to verify same user ID/name/color
4. **Integration:** Add PresenceIndicators to other views if needed
5. **Future:** When VFS sync is ready, extend store to sync across real users

---

## Notes

- Currently local-only (no VFS sync per design requirement)
- TipTap editor integration deferred until editor activity tracking spec is clear
- Cleanup timer subscriber counting assumes React StrictMode doesn't affect production
- Colors use hex values compatible with Tailwind inline styles

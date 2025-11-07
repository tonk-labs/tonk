# Presence Feature Design

**Date:** 2025-10-24 **Status:** Approved for Implementation

## Overview

Add Google Docs-style collaborative presence indicators showing who is actively viewing/editing the
document. Users see circular avatars with initials, deterministically assigned colors and animal
names, displayed in a horizontal row component.

## Requirements

### Core Functionality

- Track active users with online/offline status
- Show users active within last 5 minutes
- Display status indicators as circles with initials
- Auto-assign animal names (deterministic, no collisions needed)
- Auto-assign colors from predefined palette (deterministic)
- Single reusable component for any page location
- Local-only storage (Zustand, no VFS sync initially)

### Activity Tracking

Activity updates lastSeen timestamp on:

- Mouse movements in document area
- Keyboard input
- Editor content changes

Does NOT track:

- Window focus alone (must have interaction)

### User Display

- Show current user's circle same as others (no special "You" label)
- Hover shows animal name only (no timestamps)
- Circles display first letter of each word as initials (e.g., "EP" for "Eager Panda")

### Identity

- Browser fingerprint ID persisted in localStorage
- Stable across sessions within same browser
- Deterministic color and name derived from userId

## Data Model

### User Type

```typescript
interface User {
  id: string; // Persistent browser fingerprint UUID
  name: string; // Auto-assigned animal name (e.g., "Eager Panda")
  color: string; // Deterministically assigned from palette
  lastSeen: number; // Timestamp in milliseconds
}
```

### Store State

```typescript
interface PresenceState {
  users: Record<string, User>; // All users by ID
  currentUserId: string | null; // This user's persistent ID
}
```

### Store Actions

```typescript
{
  updatePresence: () => void;      // Update current user's lastSeen
  pruneStaleUsers: () => void;     // Remove users inactive >5min
  getActiveUsers: () => User[];    // Filter users active in last 5min
  initializeUser: () => void;      // Set up userId on first load
}
```

## Architecture

### Store Implementation

- **Pattern:** StoreBuilder with immer and persist middleware
- **Location:** `app/src/features/presence/stores/presenceStore.ts`
- **Persistence:** localStorage with key `tonk-presence`
- **Cleanup Strategy:** Centralized - setInterval runs every 30 seconds
- **Activity Updates:** Immediate (no debouncing)

### StoreBuilder Usage

```typescript
export const presenceStore = StoreBuilder(initialState, {
  name: 'tonk-presence',
  version: 1,
});

export const usePresenceStore = presenceStore.useStore;
export const usePresence = presenceStore.createFactory(createPresenceActions(presenceStore));
```

Components use:

- `usePresenceStore()` - reactive state only
- `usePresence()` - state + actions

### Color Assignment

**Deterministic hashing:**

```typescript
const getUserColor = (userId: string): string => {
  const hash = userId.split('').reduce((acc, char) => char.charCodeAt(0) + ((acc << 5) - acc), 0);
  return PRESENCE_COLORS[Math.abs(hash) % PRESENCE_COLORS.length];
};
```

**Palette:** 8 colors defined in `app/src/lib/constants/presenceColors.ts`

```typescript
export const PRESENCE_COLORS = [
  '#FF6B6B',
  '#4ECDC4',
  '#45B7D1',
  '#FFA07A',
  '#98D8C8',
  '#F7DC6F',
  '#BB8FCE',
  '#85C1E2',
] as const;
```

### Name Assignment

**Package:** `unique-names-generator` (add to dependencies)

```typescript
import { uniqueNamesGenerator, animals, adjectives } from 'unique-names-generator';

const getUserName = (userId: string): string => {
  return uniqueNamesGenerator({
    dictionaries: [adjectives, animals],
    seed: userId, // Deterministic
    separator: ' ',
    style: 'capital',
  });
};
```

### User Initialization

On store creation:

1. Check `localStorage.getItem('tonk-user-id')`
2. If missing, generate UUID and persist
3. Generate deterministic color and name from userId
4. Add current user to users map with `lastSeen: Date.now()`

### Activity Tracking

**Hook:** `usePresenceTracking()`

Throttled event listeners (using existing `lodash.throttle` dependency):

- `mousemove` on document
- `keydown` on document
- TipTap editor `onUpdate` event

Throttle: Max 1 call per 1000ms

On activity: Call `updatePresence()` to update current user's lastSeen

### Cleanup Timer

Managed in presenceStore:

- Start setInterval on first subscriber
- Run `pruneStaleUsers()` every 30 seconds
- Clear interval when last subscriber unmounts
- Store tracks subscriber count internally

## Component Design

### PresenceIndicators Component

**Location:** `app/src/features/presence/components/PresenceIndicators.tsx`

**Props:**

```typescript
interface PresenceIndicatorsProps {
  className?: string; // Optional Tailwind classes
  maxVisible?: number; // Optional limit, e.g., show max 5 + "+3 more"
}
```

**Rendering:**

- Horizontal flexbox row
- Each active user: circular avatar (32x32px default)
- Avatar: colored background + white centered text (initials)
- Initials: First letter of each word in name, uppercase
- Tooltip on hover: Full animal name
- Order: Sort by lastSeen descending (most recent first)

**Styling:**

- Use Tailwind CSS (already in project via `@tailwindcss/vite`)
- Circles: `rounded-full flex items-center justify-center`
- Slight overlap for compact display (-ml-2 for subsequent items)

### Example Usage

```tsx
import { PresenceIndicators } from '@/features/presence';

function App() {
  return (
    <div>
      <PresenceIndicators className="absolute top-4 right-4" maxVisible={5} />
      {/* Rest of app */}
    </div>
  );
}
```

## File Structure

```
app/src/
├── lib/
│   ├── storeBuilder.ts              (copied from /Users/alexander/Node/storebuilder.ts)
│   └── constants/
│       └── presenceColors.ts        (color palette constant)
├── features/
    └── presence/
        ├── index.ts                 (public exports)
        ├── stores/
        │   └── presenceStore.ts     (Zustand store with StoreBuilder)
        ├── components/
        │   └── PresenceIndicators.tsx
        ├── hooks/
        │   └── usePresenceTracking.ts
        └── utils/
            └── userGeneration.ts    (getUserColor, getUserName, initializeUserId)
```

## Implementation Notes

### Dependencies to Add

```bash
bun add unique-names-generator
```

### Existing Dependencies Used

- `zustand` (already installed)
- `zustand/middleware` (immer, persist)
- `lodash.throttle` (already installed)
- `lucide-react` (for potential icons if needed)

### Integration Points

1. Copy `storeBuilder.ts` to `app/src/lib/`
2. Activity tracking can integrate with TipTap editor via `onUpdate` prop
3. Component can be imported and placed in `App.tsx` or any layout

### Future Enhancements (Out of Scope)

- VFS sync for true multi-user collaboration
- Cursor position indicators
- User name customization
- "Who's viewing" dropdown with full list
- Activity heatmap or detailed status messages

## Success Criteria

✅ Users assigned persistent IDs across browser sessions ✅ Deterministic colors and names (same
user = same appearance) ✅ Active users (<5min) visible in component ✅ Stale users (>5min)
automatically removed ✅ Activity tracking updates presence on mouse/keyboard/editor events ✅
Component can be placed anywhere and displays horizontal avatar row ✅ Hover shows animal name ✅ No
manual user setup required (fully automatic)

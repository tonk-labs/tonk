import { FHS } from '../../../lib/paths';
import { StoreBuilder } from '../../../lib/storeBuilder';
import {
  getUserColor,
  getUserName,
  initializeUserId,
} from '../utils/userGeneration';

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

const initialState: PresenceState = {
  users: {},
  currentUserId: null,
};

// Create store with VFS sync middleware - only sync users, not currentUserId
export const presenceStore = StoreBuilder(initialState, undefined, {
  path: FHS.getServicePath('presence', 'users.json'),
  partialize: state => ({ users: state.users }), // Only sync users!
});

// Initialize current user locally after store creation
const initializeCurrentUser = () => {
  console.log('[PresenceStore] initializeCurrentUser() called');
  const userId = initializeUserId();
  console.log('[PresenceStore] Got userId from localStorage:', userId);

  const initialUser: User = {
    id: userId,
    name: getUserName(userId),
    color: getUserColor(userId),
    lastSeen: Date.now(),
  };

  const stateBefore = presenceStore.get();
  console.log('[PresenceStore] State before setting currentUserId:', {
    currentUserId: stateBefore.currentUserId,
    userCount: Object.keys(stateBefore.users).length,
  });

  presenceStore.set(state => {
    state.currentUserId = userId;
    if (!state.users[userId]) {
      state.users[userId] = initialUser;
    }
  });

  const stateAfter = presenceStore.get();
  console.log('[PresenceStore] State after setting currentUserId:', {
    currentUserId: stateAfter.currentUserId,
    userCount: Object.keys(stateAfter.users).length,
  });
};

// Call initialization immediately
initializeCurrentUser();

// Subscribe to store changes and ALWAYS ensure currentUserId matches localStorage
// This protects against VFS sync overwriting the local-only currentUserId
presenceStore.subscribe(state => {
  const currentUserId = state.currentUserId;
  const localUserId = localStorage.getItem('tonk-user-id');

  // ALWAYS ensure currentUserId matches localStorage, no matter how many times VFS syncs
  if (localUserId && currentUserId !== localUserId) {
    console.log(
      '[PresenceStore] currentUserId mismatch! Re-setting from localStorage:',
      {
        current: currentUserId,
        expected: localUserId,
      }
    );

    presenceStore.set(state => {
      state.currentUserId = localUserId;
      // Also ensure this user exists in the users map
      if (!state.users[localUserId]) {
        state.users[localUserId] = {
          id: localUserId,
          name: getUserName(localUserId),
          color: getUserColor(localUserId),
          lastSeen: Date.now(),
        };
      }
    });
  }
});

// Export base hooks
export const usePresenceStore = presenceStore.useStore;

const FIVE_MINUTES_MS = 5 * 60 * 1000;

const createPresenceActions = () => {
  const store = presenceStore;

  return {
    /**
     * Update current user's lastSeen timestamp
     * VFS sync is handled automatically by middleware
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
     * Get all users
     */
    getAllUsers: (): User[] => {
      const { users } = store.get();
      return Object.values(users);
    },

    /**
     * Get users online in last 5 minutes, sorted by lastSeen desc
     */
    getOnlineUsers: (): User[] => {
      const { users } = store.get();
      const cutoffTime = Date.now() - FIVE_MINUTES_MS;

      return Object.values(users)
        .filter(user => user.lastSeen >= cutoffTime)
        .sort((a, b) => b.lastSeen - a.lastSeen);
    },

    /**
     * Get users offline (not seen in last 5 minutes), sorted by lastSeen desc
     */
    getOfflineUsers: (): User[] => {
      const { users } = store.get();
      const cutoffTime = Date.now() - FIVE_MINUTES_MS;

      return Object.values(users)
        .filter(user => user.lastSeen < cutoffTime)
        .sort((a, b) => b.lastSeen - a.lastSeen);
    },

    getCurrentUser: (): User | null => {
      const { users, currentUserId } = store.get();
      if (!currentUserId) return null;
      return users[currentUserId] || null;
    },
  };
};

// Export factory hook
export const usePresence = presenceStore.createFactory(createPresenceActions());

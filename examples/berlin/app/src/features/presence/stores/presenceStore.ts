import { StoreBuilder } from '../../../lib/storeBuilder';
import {
  initializeUserId,
  getUserColor,
  getUserName,
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

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

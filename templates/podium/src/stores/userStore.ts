import { create } from "zustand";
import { sync, DocumentId } from "@tonk/keepsync";
import { User } from "../types/users";
import { PasskeyManager } from "../utils/passkey";

// Synced store - only contains data that should be shared across devices
interface SyncedUsersState {
  users: User[];
  addUser: (user: User) => void;
  updateUser: (
    userId: string,
    updates: Partial<Pick<User, "name" | "relationToOwner" | "profilePicture">>,
  ) => void;
  getUser: (userId: string) => User | undefined;
  isUserOwner: (userId: string) => boolean;
}

export const useSyncedUsersStore = create<SyncedUsersState>(
  sync(
    (set, get) => ({
      users: [],

      addUser: (user) => {
        set((state) => ({
          users: [...state.users, user],
        }));
      },

      updateUser: (userId, updates) => {
        set((state) => ({
          users: state.users.map((user) =>
            user.id === userId ? { ...user, ...updates } : user,
          ),
        }));
      },

      getUser: (userId) => {
        return get().users.find((u) => u.id === userId);
      },

      isUserOwner: (userId) => {
        const user = get().users.find((u) => u.id === userId);
        return user?.isOwner || false;
      },
    }),
    {
      docId: "podium/users" as DocumentId,
      initTimeout: 30000,
      onInitError: (error) =>
        console.error("User sync initialization error:", error),
    },
  ),
);

// Local store - contains UI state that should NOT be synced
interface LocalAuthState {
  currentUser: User | null;
  isAuthenticated: boolean;

  registerUser: (
    userData: Omit<User, "id" | "isOwner" | "createdAt">,
  ) => Promise<User>;
  authenticateWithPasskey: () => Promise<User | null>;
  logoutUser: () => void;
  removePasskey: () => void; // For completely removing passkey
  updateCurrentUserProfile: (
    updates: Partial<Pick<User, "name" | "relationToOwner" | "profilePicture">>,
  ) => void;
  tryAutoLogin: () => Promise<void>;
}

export const useLocalAuthStore = create<LocalAuthState>((set, get) => ({
  currentUser: null,
  isAuthenticated: false,

  registerUser: async (userData) => {
    const syncedStore = useSyncedUsersStore.getState();
    
    // First user becomes owner
    const isOwner = syncedStore.users.length === 0;
    
    // Generate user ID
    const userId = crypto.randomUUID();
    
    let credentialStored = false;
    
    // Try to create a passkey if supported
    if (PasskeyManager.isSupported()) {
      try {
        await PasskeyManager.createPasskey(userId, userData.name);
        credentialStored = true;
        console.log('✅ Passkey created and stored for userId:', userId);
      } catch (error) {
        console.warn('Failed to create passkey, using fallback:', error);
        // Store fallback credential manually
        const fallbackCredential = {
          id: userId,
          userId: userId,
          publicKey: 'fallback',
          createdAt: Date.now(),
        };
        localStorage.setItem('podium_passkey_credential', JSON.stringify(fallbackCredential));
        localStorage.setItem('podium_user_id', userId);
        credentialStored = true;
        console.log('✅ Fallback credential stored for userId:', userId);
      }
    } else {
      // Store fallback credential for unsupported browsers
      const fallbackCredential = {
        id: userId,
        userId: userId,
        publicKey: 'fallback',
        createdAt: Date.now(),
      };
      localStorage.setItem('podium_passkey_credential', JSON.stringify(fallbackCredential));
      localStorage.setItem('podium_user_id', userId);
      credentialStored = true;
      console.log('✅ Fallback credential stored (WebAuthn not supported) for userId:', userId);
    }
    
    const newUser: User = {
      ...userData,
      id: userId, // Always use the userId, not the credential id
      isOwner,
      createdAt: Date.now(),
    };

    console.log('✅ Creating user with ID:', userId, 'credential stored:', credentialStored);

    // Add to synced store
    syncedStore.addUser(newUser);

    // Set as current user locally
    set({
      currentUser: newUser,
      isAuthenticated: true,
    });

    return newUser;
  },

  authenticateWithPasskey: async () => {
    try {
      console.log('🔄 Manual passkey authentication requested...');
      const userId = await PasskeyManager.authenticateWithPasskey();
      if (userId) {
        console.log('✅ Manual auth got userId:', userId);
        const syncedStore = useSyncedUsersStore.getState();
        const user = syncedStore.getUser(userId);
        
        if (user) {
          console.log('✅ Manual auth successful for user:', user.name);
          set({
            currentUser: user,
            isAuthenticated: true,
          });
          return user;
        } else {
          console.log('❌ User not found in synced store for userId:', userId);
          console.log('Available users:', syncedStore.users.map(u => ({ id: u.id, name: u.name })));
        }
      } else {
        console.log('❌ No userId returned from passkey authentication');
      }
    } catch (error) {
      console.error('❌ Manual passkey authentication failed:', error);
    }
    return null;
  },

  logoutUser: () => {
    // Don't clear credentials on logout - passkeys should persist!
    // PasskeyManager.clearCredentials(); // Removed this line
    console.log('🚪 Logging out user, but keeping passkey for future logins');
    set({
      currentUser: null,
      isAuthenticated: false,
    });
  },

  removePasskey: () => {
    // This completely removes the passkey - use with caution!
    console.log('🗑️ Completely removing passkey credentials');
    PasskeyManager.clearCredentials();
    set({
      currentUser: null,
      isAuthenticated: false,
    });
  },

  updateCurrentUserProfile: (updates) => {
    const currentUser = get().currentUser;
    if (currentUser) {
      const syncedStore = useSyncedUsersStore.getState();

      // Update in synced store
      syncedStore.updateUser(currentUser.id, updates);

      // Update local current user
      set({
        currentUser: { ...currentUser, ...updates },
      });
    }
  },

  tryAutoLogin: async () => {
    if (!get().isAuthenticated) {
      try {
        console.log('🔄 Trying auto-login...');
        // Try passkey authentication first
        const userId = await PasskeyManager.authenticateWithPasskey();
        if (userId) {
          console.log('✅ Auto-login got userId:', userId);
          const syncedStore = useSyncedUsersStore.getState();
          const user = syncedStore.getUser(userId);
          
          if (user) {
            console.log('✅ Auto-login successful for user:', user.name);
            set({
              currentUser: user,
              isAuthenticated: true,
            });
          } else {
            console.log('❌ User not found in synced store for userId:', userId);
          }
        } else {
          console.log('ℹ️ No auto-login available');
        }
      } catch (error) {
        console.error('❌ Auto-login failed:', error);
      }
    }
  },
}));


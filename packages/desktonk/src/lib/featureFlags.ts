import { create } from 'zustand';
import { persist } from 'zustand/middleware';

export interface FeatureFlags {
  minimalDesktopUI: boolean;
  lockCamera: boolean;
  showMembersBar: boolean;
}

interface FeatureFlagStore {
  flags: FeatureFlags;
  setFlag: (key: keyof FeatureFlags, value: boolean) => void;
  resetFlags: () => void;
}

const DEFAULT_FLAGS: FeatureFlags = {
  minimalDesktopUI: true, // Default to normal UI, can be toggled
  lockCamera: true, // Default to camera enabled, can be locked
  showMembersBar: true,
};

const STORAGE_VERSION = 1;

export const useFeatureFlagStore = create<FeatureFlagStore>()(
  persist(
    (set) => ({
      flags: DEFAULT_FLAGS,
      setFlag: (key, value) =>
        set((state) => ({
          flags: { ...state.flags, [key]: value },
        })),
      resetFlags: () => set({ flags: DEFAULT_FLAGS }),
    }),
    {
      name: 'desktonk-feature-flags',
      version: STORAGE_VERSION,
      migrate: (persistedState: unknown, version: number) => {
        // Migrate old state to include any new flags with defaults
        if (version < STORAGE_VERSION) {
          const state = persistedState as { flags?: FeatureFlags };
          return {
            ...state,
            flags: {
              ...DEFAULT_FLAGS,
              ...state.flags,
            },
          };
        }
        return persistedState as FeatureFlagStore;
      },
    }
  )
);

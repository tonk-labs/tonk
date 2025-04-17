import { create } from "zustand";
import { DocumentId, sync } from "@tonk/keepsync";
import { useLocationStore } from "./locationStore";

export interface UserProfile {
  id: string;
  name: string;
  createdAt: number;
}

interface UserState {
  profiles: UserProfile[];
  activeProfileId: string | null;
  createProfile: (name: string) => UserProfile;
  setActiveProfile: (id: string) => void;
  updateProfileName: (id: string, name: string) => void;
  deleteProfile: (id: string) => void;
}

// Generate a random ID
const generateId = () => Math.random().toString(36).substring(2, 15);

// Get the active profile ID from localStorage
const getActiveProfileId = () => {
  return localStorage.getItem("activeProfileId");
};

export const useUserStore = create<UserState>(
  sync(
    (set) => ({
      profiles: [],
      activeProfileId: getActiveProfileId(),

      createProfile: (name) => {
        const id = generateId();
        const newProfile: UserProfile = {
          id,
          name,
          createdAt: Date.now(),
        };

        set((state) => ({
          profiles: [...state.profiles, newProfile],
        }));

        // Update the user name in the location store's map
        useLocationStore.getState().updateUserName(id, name);

        return newProfile;
      },

      setActiveProfile: (id) => {
        // Store the active profile ID in localStorage
        localStorage.setItem("activeProfileId", id);

        set({ activeProfileId: id });
      },

      updateProfileName: (id, name) => {
        set((state) => ({
          profiles: state.profiles.map((profile) =>
            profile.id === id ? { ...profile, name } : profile,
          ),
        }));

        // Update the user name in the location store's map
        useLocationStore.getState().updateUserName(id, name);
      },

      deleteProfile: (id) => {
        set((state) => {
          // Filter out the profile to delete
          const updatedProfiles = state.profiles.filter(
            (profile) => profile.id !== id,
          );

          // If we're deleting the active profile, set activeProfileId to null
          const newActiveProfileId =
            state.activeProfileId === id
              ? updatedProfiles.length > 0
                ? updatedProfiles[0].id
                : null
              : state.activeProfileId;

          // Update localStorage if active profile changed
          if (newActiveProfileId !== state.activeProfileId) {
            if (newActiveProfileId) {
              localStorage.setItem("activeProfileId", newActiveProfileId);
            } else {
              localStorage.removeItem("activeProfileId");
            }
          }

          return {
            profiles: updatedProfiles,
            activeProfileId: newActiveProfileId,
          };
        });
      },
    }),
    {
      docId: "my-world-users" as DocumentId,
      initTimeout: 30000,
      onInitError: (error) =>
        console.error("User sync initialization error:", error),
    },
  ),
);

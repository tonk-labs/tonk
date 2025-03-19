import { create } from "zustand";
import { sync } from "@tonk/keepsync";
import { useUserStore } from "./userStore";

export interface Location {
  id: string;
  name: string;
  description: string;
  latitude: number;
  longitude: number;
  addedBy: string;
  createdAt: number;
}

export interface LocationState {
  locations: Record<string, Location>;
  // Add a map of user IDs to names
  userNames: Record<string, string>;

  // Actions
  addLocation: (
    location: Omit<Location, "id" | "addedBy" | "createdAt">,
  ) => void;
  removeLocation: (id: string) => void;
  updateLocation: (
    id: string,
    updates: Partial<Omit<Location, "id" | "addedBy" | "createdAt">>,
  ) => void;
  // Add a new action to update the user name mapping
  updateUserName: (userId: string, name: string) => void;
}

// Generate a random ID
const generateId = () => Math.random().toString(36).substring(2, 15);

export const useLocationStore = create<LocationState>(
  sync(
    (set) => ({
      // State
      locations: {},
      userNames: {}, // Initialize empty map of user IDs to names

      // Actions
      addLocation: (locationData) => {
        set((state) => {
          const id = generateId();
          // Get the current user ID from the userStore
          const { id: userId, name: userName } =
            useUserStore.getState().profile;

          // Update the userNames map with the current user's name
          const userNames = {
            ...state.userNames,
            [userId]: userName,
          };

          const location: Location = {
            ...locationData,
            id,
            addedBy: userId,
            createdAt: Date.now(),
          };

          return {
            locations: {
              ...state.locations,
              [id]: location,
            },
            userNames, // Include updated userNames in the state update
          };
        });
      },

      // Add function to update a user's name in the map
      updateUserName: (userId, name) => {
        set((state) => ({
          userNames: {
            ...state.userNames,
            [userId]: name,
          },
        }));
      },

      removeLocation: (id) => {
        set((state) => {
          const newLocations = { ...state.locations };
          delete newLocations[id];
          return { locations: newLocations };
        });
      },

      updateLocation: (id, updates) => {
        set((state) => {
          if (!state.locations[id]) return state;

          return {
            locations: {
              ...state.locations,
              [id]: {
                ...state.locations[id],
                ...updates,
              },
            },
          };
        });
      },
    }),
    {
      docId: "my-world-locations",
      initTimeout: 30000,
      onInitError: (error) =>
        console.error("Location sync initialization error:", error),
    },
  ),
);

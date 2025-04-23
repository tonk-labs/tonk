import { create } from "zustand";
import { DocumentId, sync } from "@tonk/keepsync";
import { useUserStore } from "./userStore";

export interface Review {
  id: string;
  userId: string;
  rating: number;
  comment: string;
  createdAt: number;
}

export interface Location {
  id: string;
  name: string;
  description: string;
  latitude: number;
  longitude: number;
  addedBy: string;
  createdAt: number;
  placeId: string;
  category: string;
  isOpen?: boolean | null;
  reviews?: Review[];
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
  updateUserName: (userId: string, name: string) => void;
  addReview: (locationId: string, rating: number, comment: string) => void;
  removeReview: (locationId: string, reviewId: string) => void;
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

          // Get the current active user ID from the userStore
          const userStore = useUserStore.getState();
          const activeProfileId = userStore.activeProfileId;

          if (!activeProfileId) {
            console.error("No active user profile found");
            return state;
          }

          // Find the active profile
          const activeProfile = userStore.profiles.find(
            (profile) => profile.id === activeProfileId,
          );

          if (!activeProfile) {
            console.error("Active profile not found");
            return state;
          }

          // Update the userNames map with the current user's name
          const userNames = {
            ...state.userNames,
            [activeProfileId]: activeProfile.name,
          };

          const location: Location = {
            ...locationData,
            id,
            addedBy: activeProfileId,
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

      addReview: (locationId, rating, comment) => {
        set((state) => {
          if (!state.locations[locationId]) return state;

          // Get the current active user ID from the userStore
          const userStore = useUserStore.getState();
          const activeProfileId = userStore.activeProfileId;

          if (!activeProfileId) {
            console.error("No active user profile found");
            return state;
          }

          const reviewId = generateId();
          const review: Review = {
            id: reviewId,
            userId: activeProfileId,
            rating,
            comment,
            createdAt: Date.now(),
          };

          const location = state.locations[locationId];
          const reviews = location.reviews
            ? [...location.reviews, review]
            : [review];

          return {
            locations: {
              ...state.locations,
              [locationId]: {
                ...location,
                reviews,
              },
            },
          };
        });
      },

      removeReview: (locationId, reviewId) => {
        set((state) => {
          if (
            !state.locations[locationId] ||
            !state.locations[locationId].reviews
          )
            return state;

          const location = state.locations[locationId];
          const reviews = location.reviews?.filter(
            (review) => review.id !== reviewId,
          );

          return {
            locations: {
              ...state.locations,
              [locationId]: {
                ...location,
                reviews,
              },
            },
          };
        });
      },
    }),
    {
      docId: "my-world-locations" as DocumentId,
      initTimeout: 30000,
      onInitError: (error) =>
        console.error("Location sync initialization error:", error),
    },
  ),
);

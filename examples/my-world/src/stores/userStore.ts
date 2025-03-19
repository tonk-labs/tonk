import { create } from "zustand";
import { useLocationStore } from "./locationStore";

interface UserState {
  profile: {
    name: string;
    id: string;
  };
  setUserProfile: (name: string) => void;
}

// Generate a random ID
const generateId = () => Math.random().toString(36).substring(2, 15);

// Generate a random user ID if none exists in localStorage
const getUserId = () => {
  const storedId = localStorage.getItem("userId");
  if (storedId) return storedId;

  const newId = generateId();
  localStorage.setItem("userId", newId);
  return newId;
};

// Initialize with default user profile
const initialUserId = getUserId();
const initialUserName = localStorage.getItem("userName") || "Anonymous";

export const useUserStore = create<UserState>((set) => ({
  profile: {
    name: initialUserName,
    id: initialUserId,
  },

  setUserProfile: (name) => {
    localStorage.setItem("userName", name);
    const userId = getUserId();

    // Update the user name in the location store's map
    useLocationStore.getState().updateUserName(userId, name);

    set((state) => ({
      profile: {
        ...state.profile,
        name,
      },
    }));
  },
}));

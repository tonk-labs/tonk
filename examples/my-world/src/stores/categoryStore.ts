import { create } from "zustand";
import { DocumentId, sync } from "@tonk/keepsync";
import { useUserStore } from "./userStore";

export interface Category {
  id: string;
  name: string;
  color: string;
  icon?: string | null;
  createdBy: string;
}

export interface CategoryState {
  categories: Record<string, Category>;

  // Actions
  addCategory: (name: string, color: string, icon?: string | null) => Category;
  updateCategory: (
    id: string,
    updates: Partial<Omit<Category, "id" | "createdBy">>,
  ) => void;
  deleteCategory: (id: string) => void;
}

// Generate a random ID
const generateId = () => Math.random().toString(36).substring(2, 15);

// Default categories with Apple-style colors
const defaultCategories: Category[] = [
  {
    id: "default",
    name: "General",
    color: "#8E8E93",
    icon: "map-pin",
    createdBy: "system",
  },
  {
    id: "favorite",
    name: "Favorites",
    color: "#FF9500",
    icon: "star",
    createdBy: "system",
  },
  {
    id: "food",
    name: "Food & Drink",
    color: "#FF2D55",
    icon: "utensils",
    createdBy: "system",
  },
  {
    id: "shopping",
    name: "Shopping",
    color: "#5AC8FA",
    icon: "shopping-bag",
    createdBy: "system",
  },
  {
    id: "nature",
    name: "Nature",
    color: "#34C759",
    icon: "tree",
    createdBy: "system",
  },
  {
    id: "culture",
    name: "Culture",
    color: "#AF52DE",
    icon: "landmark",
    createdBy: "system",
  },
];

export const useCategoryStore = create<CategoryState>(
  sync(
    (set, _get) => ({
      // State
      categories: defaultCategories.reduce(
        (acc, category) => {
          acc[category.id] = category;
          return acc;
        },
        {} as Record<string, Category>,
      ),

      // Actions
      addCategory: (name, color, icon) => {
        const id = generateId();
        const activeProfileId = useUserStore.getState().activeProfileId;

        if (!activeProfileId) {
          console.error("No active user profile found");
          throw new Error("No active user profile found");
        }

        const newCategory: Category = {
          id,
          name,
          color,
          // Ensure icon is never undefined by defaulting to null
          icon: icon === undefined ? null : icon,
          createdBy: activeProfileId,
        };

        set((state) => ({
          categories: {
            ...state.categories,
            [id]: newCategory,
          },
        }));

        return newCategory;
      },

      updateCategory: (id, updates) => {
        set((state) => {
          if (!state.categories[id]) return state;

          // Don't allow modifying system categories
          if (state.categories[id].createdBy === "system") {
            console.warn("Cannot modify system categories");
            return state;
          }

          // Ensure icon is never undefined
          const safeUpdates = { ...updates };
          if ("icon" in safeUpdates && safeUpdates.icon === undefined) {
            safeUpdates.icon = null;
          }

          return {
            categories: {
              ...state.categories,
              [id]: {
                ...state.categories[id],
                ...safeUpdates,
              },
            },
          };
        });
      },

      deleteCategory: (id) => {
        set((state) => {
          // Don't allow deleting system categories
          if (state.categories[id]?.createdBy === "system") {
            console.warn("Cannot delete system categories");
            return state;
          }

          const newCategories = { ...state.categories };
          delete newCategories[id];
          return { categories: newCategories };
        });
      },
    }),
    {
      docId: "my-world-categories" as DocumentId,
      initTimeout: 30000,
      onInitError: (error) =>
        console.error("Category sync initialization error:", error),
    },
  ),
);

import { sync } from "@tonk/keepsync";
import { create } from "zustand";

// Define the Todo type with additional properties for a more robust app
export interface Todo {
  id: string;
  text: string;
  completed: boolean;
  createdAt: number;
  priority: "low" | "medium" | "high";
  dueDate?: string; // Optional due date
}

// Define the filter options
export type FilterOption = "all" | "active" | "completed";

// Define the sort options
export type SortOption = "createdAt" | "priority" | "dueDate";

// Define the data structure
interface TodoData {
  todos: Todo[];
  currentFilter: FilterOption;
  currentSort: SortOption;
  sortDirection: "asc" | "desc";
}

// Define the store state with more sophisticated functionality
interface TodoState extends TodoData {
  // Core CRUD operations
  addTodo: (text: string, priority?: Todo["priority"], dueDate?: string) => void;
  toggleTodo: (id: string) => void;
  deleteTodo: (id: string) => void;
  updateTodoText: (id: string, text: string) => void;
  updateTodoPriority: (id: string, priority: Todo["priority"]) => void;
  updateTodoDueDate: (id: string, dueDate?: string) => void;
  
  // Batch operations
  clearCompleted: () => void;
  deleteAll: () => void;
  
  // Filtering and sorting operations
  setFilter: (filter: FilterOption) => void;
  setSort: (sort: SortOption) => void;
  toggleSortDirection: () => void;
  
  // Computed properties for the UI
  getFilteredAndSortedTodos: () => Todo[];
  getRemainingCount: () => number;
  getCompletedCount: () => number;
}

// Create a synced store for todos
export const useTodoStore = create<TodoState>(
  sync(
    (set, get) => ({
      todos: [],
      currentFilter: "all",
      currentSort: "createdAt",
      sortDirection: "desc", // Newest first by default

      // Core CRUD operations
      addTodo: (text, priority = "medium", dueDate) => {
        set((state) => ({
          todos: [
            ...state.todos,
            {
              id: crypto.randomUUID(),
              text,
              completed: false,
              createdAt: Date.now(),
              priority,
              dueDate,
            },
          ],
        }));
      },

      toggleTodo: (id) => {
        set((state) => ({
          todos: state.todos.map((todo) =>
            todo.id === id ? { ...todo, completed: !todo.completed } : todo
          ),
        }));
      },

      deleteTodo: (id) => {
        set((state) => ({
          todos: state.todos.filter((todo) => todo.id !== id),
        }));
      },

      updateTodoText: (id, text) => {
        set((state) => ({
          todos: state.todos.map((todo) =>
            todo.id === id ? { ...todo, text } : todo
          ),
        }));
      },

      updateTodoPriority: (id, priority) => {
        set((state) => ({
          todos: state.todos.map((todo) =>
            todo.id === id ? { ...todo, priority } : todo
          ),
        }));
      },

      updateTodoDueDate: (id, dueDate) => {
        set((state) => ({
          todos: state.todos.map((todo) =>
            todo.id === id ? { ...todo, dueDate } : todo
          ),
        }));
      },

      // Batch operations
      clearCompleted: () => {
        set((state) => ({
          todos: state.todos.filter((todo) => !todo.completed),
        }));
      },

      deleteAll: () => {
        set({ todos: [] });
      },

      // Filtering and sorting operations
      setFilter: (filter) => {
        set({ currentFilter: filter });
      },

      setSort: (sort) => {
        set({ currentSort: sort });
      },

      toggleSortDirection: () => {
        set((state) => ({
          sortDirection: state.sortDirection === "asc" ? "desc" : "asc",
        }));
      },

      // Computed methods for the UI
      getFilteredAndSortedTodos: () => {
        const { todos, currentFilter, currentSort, sortDirection } = get();
        
        // First filter the todos
        const filteredTodos = todos.filter((todo) => {
          if (currentFilter === "all") return true;
          if (currentFilter === "active") return !todo.completed;
          if (currentFilter === "completed") return todo.completed;
          return true;
        });
        
        // Then sort them
        return filteredTodos.sort((a, b) => {
          let comparison = 0;
          
          if (currentSort === "createdAt") {
            comparison = a.createdAt - b.createdAt;
          } else if (currentSort === "priority") {
            const priorityValues = { low: 0, medium: 1, high: 2 };
            comparison = priorityValues[a.priority] - priorityValues[b.priority];
          } else if (currentSort === "dueDate") {
            // Handle undefined due dates - put them at the end
            if (!a.dueDate && !b.dueDate) {
              comparison = 0;
            } else if (!a.dueDate) {
              comparison = 1;
            } else if (!b.dueDate) {
              comparison = -1;
            } else {
              comparison = new Date(a.dueDate).getTime() - new Date(b.dueDate).getTime();
            }
          }
          
          // Reverse the comparison for descending order
          return sortDirection === "asc" ? comparison : -comparison;
        });
      },

      getRemainingCount: () => {
        return get().todos.filter((todo) => !todo.completed).length;
      },

      getCompletedCount: () => {
        return get().todos.filter((todo) => todo.completed).length;
      },
    }),
    {
      // Unique document ID for this store
      docId: "todo-list",
    }
  )
);
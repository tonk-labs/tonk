import { type DocumentId, sync } from '@tonk/keepsync';
import { create } from 'zustand';

// Define the Todo type
export interface Todo {
  id: string;
  text: string;
  completed: boolean;
}

// Define the data structure
interface TodoData {
  todos: Todo[];
}

// Define the store state
interface TodoState extends TodoData {
  addTodo: (text: string) => void;
  toggleTodo: (id: string) => void;
  deleteTodo: (id: string) => void;
}

// Create a synced store for todos
export const useTodoStore = create<TodoState>(
  sync(
    set => ({
      todos: [],

      // Add a new todo
      addTodo: (text: string) => {
        set(state => ({
          todos: [
            ...state.todos,
            {
              id: crypto.randomUUID(),
              text,
              completed: false,
            },
          ],
        }));
      },

      // Toggle a todo's completed status
      toggleTodo: (id: string) => {
        set(state => ({
          todos: state.todos.map(todo =>
            todo.id === id ? { ...todo, completed: !todo.completed } : todo
          ),
        }));
      },

      // Delete a todo
      deleteTodo: (id: string) => {
        set(state => ({
          todos: state.todos.filter(todo => todo.id !== id),
        }));
      },
    }),
    {
      // Unique document ID for this store
      docId: 'todo-list' as DocumentId,
    }
  )
);

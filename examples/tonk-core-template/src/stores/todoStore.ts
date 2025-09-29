import { create } from 'zustand';
import { TonkCore } from '@tonk/core';

export interface Todo {
  id: string;
  text: string;
  completed: boolean;
  createdAt: number;
}

interface TodoState {
  todos: Todo[];
  tonk: TonkCore | null;
  isInitialized: boolean;
  addTodo: (text: string) => Promise<void>;
  toggleTodo: (id: string) => Promise<void>;
  deleteTodo: (id: string) => Promise<void>;
  initialize: () => Promise<void>;
  connectSync: (url: string) => Promise<void>;
}

const TODOS_FILE = '/todos';

export const useTodoStore = create<TodoState>((set, get) => ({
  todos: [],
  tonk: null,
  isInitialized: false,

  initialize: async () => {
    try {
      const response = await fetch('http://localhost:6080/.manifest.tonk');
      const arrayBuffer = await response.arrayBuffer();
      const bytes = new Uint8Array(arrayBuffer);

      const tonk = await TonkCore.fromBytes(bytes, {
        storage: { type: 'indexeddb' },
      });

      await tonk.connectWebsocket('ws://localhost:6080');
      await new Promise(resolve => setTimeout(resolve, 100));

      // Load existing todos from tonk
      let todos: Todo[] = [];
      try {
        const exists = await tonk.exists(TODOS_FILE);

        if (exists) {
          const doc = await tonk.readFile(TODOS_FILE);
          const content = doc.content;
          todos = JSON.parse(content);
        } else {
          console.log('No existing todos file, starting with empty array');
          await tonk.createFile(TODOS_FILE, todos);
        }
      } catch (error) {
        console.warn('Could not load existing todos:', error);
      }

      // Set up directory watcher for reactive updates
      await tonk.watchDirectory('/', async () => {
        // Check if the event is related to our todos file
        try {
          const doc = await tonk.readFile(TODOS_FILE);
          const content = doc.content;

          let updatedTodos: Todo[] = JSON.parse(content);

          set({ todos: updatedTodos });
        } catch (error) {
          console.error('Failed to load updated todos:', error);
        }
      });

      set({ tonk, todos, isInitialized: true });
    } catch (error) {
      console.error('Failed to initialize TonkCore:', error);
      set({ isInitialized: true }); // Set initialized even on error so UI doesn't hang
    }
  },

  connectSync: async (url: string) => {
    const { tonk } = get();
    if (tonk) {
      try {
        await tonk.connectWebsocket(url);
      } catch (error) {
        console.error('Failed to connect to sync server:', error);
      }
    }
  },

  addTodo: async (text: string) => {
    const { todos, tonk } = get();
    if (!tonk) return;

    const newTodo: Todo = {
      id: crypto.randomUUID(),
      text,
      completed: false,
      createdAt: Date.now(),
    };

    const updatedTodos = [...todos, newTodo];

    try {
      await tonk.updateFile(TODOS_FILE, updatedTodos);
      set({ todos: updatedTodos });
    } catch (error) {
      console.error('Failed to save todo:', error);
    }
  },

  toggleTodo: async (id: string) => {
    const { todos, tonk } = get();
    if (!tonk) return;

    const updatedTodos = todos.map(todo =>
      todo.id === id ? { ...todo, completed: !todo.completed } : todo
    );

    try {
      await tonk.updateFile(TODOS_FILE, updatedTodos);
      set({ todos: updatedTodos });
    } catch (error) {
      console.error('Failed to update todo:', error);
    }
  },

  deleteTodo: async (id: string) => {
    const { todos, tonk } = get();
    if (!tonk) return;

    const updatedTodos = todos.filter(todo => todo.id !== id);

    try {
      await tonk.updateFile(TODOS_FILE, updatedTodos);
      set({ todos: updatedTodos });
    } catch (error) {
      console.error('Failed to delete todo:', error);
    }
  },
}));

import { create } from 'zustand';
import { TonkCore, VirtualFileSystem } from '@tonk/core';

export interface Todo {
  id: string;
  text: string;
  completed: boolean;
  createdAt: number;
}

interface TodoState {
  todos: Todo[];
  tonk: TonkCore | null;
  vfs: VirtualFileSystem | null;
  isInitialized: boolean;
  addTodo: (text: string) => Promise<void>;
  toggleTodo: (id: string) => Promise<void>;
  deleteTodo: (id: string) => Promise<void>;
  initialize: () => Promise<void>;
  connectSync: (url: string) => Promise<void>;
}

const TODOS_FILE = '/todos.json';

export const useTodoStore = create<TodoState>((set, get) => ({
  todos: [],
  tonk: null,
  vfs: null,
  isInitialized: false,

  initialize: async () => {
    try {
      console.log('Initializing TonkCore with IndexedDB...');
      const tonk = await TonkCore.create({ storage: { type: 'indexeddb' } });
      const vfs = await tonk.getVfs();

      console.log('TonkCore initialized, checking for existing todos...');

      // Load existing todos from VFS
      let todos: Todo[] = [];
      try {
        const exists = await vfs.exists(TODOS_FILE);
        console.log(`Todos file exists: ${exists}`);

        if (exists) {
          const content = await vfs.readFile(TODOS_FILE);
          console.log('Loaded todos content:', content);
          todos = JSON.parse(content);
          console.log('Parsed todos:', todos);
        } else {
          console.log('No existing todos file, starting with empty array');
        }
      } catch (error) {
        console.warn('Could not load existing todos:', error);
      }

      set({ tonk, vfs, todos, isInitialized: true });
      console.log('Todo store initialized with', todos.length, 'todos');
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
        console.log('Connected to sync server:', url);
      } catch (error) {
        console.error('Failed to connect to sync server:', error);
      }
    }
  },

  addTodo: async (text: string) => {
    const { todos, vfs } = get();
    if (!vfs) return;

    const newTodo: Todo = {
      id: crypto.randomUUID(),
      text,
      completed: false,
      createdAt: Date.now(),
    };

    const updatedTodos = [...todos, newTodo];

    try {
      await vfs.createFile(TODOS_FILE, JSON.stringify(updatedTodos, null, 2));
      set({ todos: updatedTodos });
    } catch (error) {
      console.error('Failed to save todo:', error);
    }
  },

  toggleTodo: async (id: string) => {
    const { todos, vfs } = get();
    if (!vfs) return;

    const updatedTodos = todos.map(todo =>
      todo.id === id ? { ...todo, completed: !todo.completed } : todo
    );

    try {
      await vfs.createFile(TODOS_FILE, JSON.stringify(updatedTodos, null, 2));
      set({ todos: updatedTodos });
    } catch (error) {
      console.error('Failed to update todo:', error);
    }
  },

  deleteTodo: async (id: string) => {
    const { todos, vfs } = get();
    if (!vfs) return;

    const updatedTodos = todos.filter(todo => todo.id !== id);

    try {
      await vfs.createFile(TODOS_FILE, JSON.stringify(updatedTodos, null, 2));
      set({ todos: updatedTodos });
    } catch (error) {
      console.error('Failed to delete todo:', error);
    }
  },
}));


export interface StoreTemplate {
  name: string;
  description: string;
  code: string;
}

export const STORE_TEMPLATES: StoreTemplate[] = [
  {
    name: 'CounterStore',
    description: 'Simple counter with increment/decrement actions',
    code: `// create and sync are available in the compilation context
interface CounterState {
  count: number;
  increment: () => void;
  decrement: () => void;
  reset: () => void;
  setCount: (value: number) => void;
}

const useCounterStore = create<CounterState>()(
  sync(
    (set) => ({
      count: 0,
      increment: () => set((state) => ({ count: state.count + 1 })),
      decrement: () => set((state) => ({ count: state.count - 1 })),
      reset: () => set({ count: 0 }),
      setCount: (value) => set({ count: value }),
    }),
    { path: '/src/stores/counter-store.json' }
  )
);

export default useCounterStore;`,
  },
  {
    name: 'AuthStore',
    description: 'User authentication and session management',
    code: `// create and sync are available in the compilation context
interface User {
  id: string;
  email: string;
  name: string;
}

interface AuthState {
  user: User | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  login: (email: string, password: string) => Promise<void>;
  logout: () => void;
  setUser: (user: User | null) => void;
  setLoading: (loading: boolean) => void;
}

const useAuthStore = create<AuthState>()(
  sync(
    (set, get) => ({
      user: null,
      isAuthenticated: false,
      isLoading: false,
      
      login: async (email: string, password: string) => {
        set({ isLoading: true });
        try {
          // Simulate API call
          await new Promise(resolve => setTimeout(resolve, 1000));
          const user = { 
            id: '1', 
            email, 
            name: email.split('@')[0] 
          };
          set({ 
            user, 
            isAuthenticated: true, 
            isLoading: false 
          });
        } catch (error) {
          set({ isLoading: false });
          throw error;
        }
      },
      
      logout: () => {
        set({ 
          user: null, 
          isAuthenticated: false 
        });
      },
      
      setUser: (user) => {
        set({ 
          user, 
          isAuthenticated: !!user 
        });
      },
      
      setLoading: (loading) => {
        set({ isLoading: loading });
      },
    }),
    { path: '/src/stores/auth-store.json' }
  )
);

export default useAuthStore;`,
  },
  {
    name: 'TodoStore',
    description: 'Todo list management with CRUD operations',
    code: `// create and sync are available in the compilation context
interface Todo {
  id: string;
  text: string;
  completed: boolean;
  createdAt: Date;
}

interface TodoState {
  todos: Todo[];
  filter: 'all' | 'active' | 'completed';
  addTodo: (text: string) => void;
  toggleTodo: (id: string) => void;
  deleteTodo: (id: string) => void;
  editTodo: (id: string, text: string) => void;
  setFilter: (filter: 'all' | 'active' | 'completed') => void;
  clearCompleted: () => void;
  getFilteredTodos: () => Todo[];
}

const useTodoStore = create<TodoState>()(
  sync(
    (set, get) => ({
      todos: [],
      filter: 'all',
      
      addTodo: (text: string) => {
        const newTodo: Todo = {
          id: Date.now().toString(),
          text: text.trim(),
          completed: false,
          createdAt: new Date(),
        };
        set((state) => ({
          todos: [...state.todos, newTodo]
        }));
      },
      
      toggleTodo: (id: string) => {
        set((state) => ({
          todos: state.todos.map(todo =>
            todo.id === id ? { ...todo, completed: !todo.completed } : todo
          )
        }));
      },
      
      deleteTodo: (id: string) => {
        set((state) => ({
          todos: state.todos.filter(todo => todo.id !== id)
        }));
      },
      
      editTodo: (id: string, text: string) => {
        set((state) => ({
          todos: state.todos.map(todo =>
            todo.id === id ? { ...todo, text: text.trim() } : todo
          )
        }));
      },
      
      setFilter: (filter) => {
        set({ filter });
      },
      
      clearCompleted: () => {
        set((state) => ({
          todos: state.todos.filter(todo => !todo.completed)
        }));
      },
      
      getFilteredTodos: () => {
        const { todos, filter } = get();
        switch (filter) {
          case 'active':
            return todos.filter(todo => !todo.completed);
          case 'completed':
            return todos.filter(todo => todo.completed);
          default:
            return todos;
        }
      },
    }),
    { path: '/src/stores/todo-store.json' }
  )
);

export default useTodoStore;`,
  },
  {
    name: 'ThemeStore',
    description: 'UI theme and appearance management',
    code: `// create and sync are available in the compilation context
type Theme = 'light' | 'dark' | 'system';
type ColorScheme = 'blue' | 'green' | 'purple' | 'orange';

interface ThemeState {
  theme: Theme;
  colorScheme: ColorScheme;
  fontSize: number;
  sidebarCollapsed: boolean;
  setTheme: (theme: Theme) => void;
  setColorScheme: (scheme: ColorScheme) => void;
  setFontSize: (size: number) => void;
  toggleSidebar: () => void;
  getComputedTheme: () => 'light' | 'dark';
}

const useThemeStore = create<ThemeState>()(
  sync(
    (set, get) => ({
      theme: 'system',
      colorScheme: 'blue',
      fontSize: 14,
      sidebarCollapsed: false,
      
      setTheme: (theme) => {
        set({ theme });
        // Apply theme to document
        const computedTheme = get().getComputedTheme();
        document.documentElement.setAttribute('data-theme', computedTheme);
      },
      
      setColorScheme: (colorScheme) => {
        set({ colorScheme });
        document.documentElement.setAttribute('data-color-scheme', colorScheme);
      },
      
      setFontSize: (fontSize) => {
        set({ fontSize });
        document.documentElement.style.fontSize = \`\${fontSize}px\`;
      },
      
      toggleSidebar: () => {
        set((state) => ({ 
          sidebarCollapsed: !state.sidebarCollapsed 
        }));
      },
      
      getComputedTheme: () => {
        const { theme } = get();
        if (theme === 'system') {
          return window.matchMedia('(prefers-color-scheme: dark)').matches 
            ? 'dark' 
            : 'light';
        }
        return theme;
      },
    }),
    { path: '/src/stores/theme-store.json' }
  )
);

export default useThemeStore;`,
  },
  {
    name: 'NotificationStore',
    description: 'Toast notifications and alerts management',
    code: `// create and sync are available in the compilation context
type NotificationType = 'success' | 'error' | 'warning' | 'info';

interface Notification {
  id: string;
  type: NotificationType;
  title: string;
  message?: string;
  duration?: number;
  persistent?: boolean;
  createdAt: Date;
}

interface NotificationState {
  notifications: Notification[];
  addNotification: (notification: Omit<Notification, 'id' | 'createdAt'>) => string;
  removeNotification: (id: string) => void;
  clearAll: () => void;
  success: (title: string, message?: string, duration?: number) => string;
  error: (title: string, message?: string, persistent?: boolean) => string;
  warning: (title: string, message?: string, duration?: number) => string;
  info: (title: string, message?: string, duration?: number) => string;
}

const useNotificationStore = create<NotificationState>()(
  sync(
    (set, get) => ({
      notifications: [],
      
      addNotification: (notification) => {
        const id = Date.now().toString() + Math.random().toString(36).substring(2);
        const newNotification: Notification = {
          ...notification,
          id,
          createdAt: new Date(),
          duration: notification.duration ?? 5000,
        };
        
        set((state) => ({
          notifications: [...state.notifications, newNotification]
        }));
        
        // Auto-remove after duration (unless persistent)
        if (!notification.persistent && newNotification.duration > 0) {
          setTimeout(() => {
            get().removeNotification(id);
          }, newNotification.duration);
        }
        
        return id;
      },
      
      removeNotification: (id) => {
        set((state) => ({
          notifications: state.notifications.filter(n => n.id !== id)
        }));
      },
      
      clearAll: () => {
        set({ notifications: [] });
      },
      
      success: (title, message, duration = 5000) => {
        return get().addNotification({ 
          type: 'success', 
          title, 
          message, 
          duration 
        });
      },
      
      error: (title, message, persistent = false) => {
        return get().addNotification({ 
          type: 'error', 
          title, 
          message, 
          persistent,
          duration: persistent ? 0 : 0
        });
      },
      
      warning: (title, message, duration = 7000) => {
        return get().addNotification({ 
          type: 'warning', 
          title, 
          message, 
          duration 
        });
      },
      
      info: (title, message, duration = 5000) => {
        return get().addNotification({ 
          type: 'info', 
          title, 
          message, 
          duration 
        });
      },
    }),
    { path: '/src/stores/notification-store.json' }
  )
);

export default useNotificationStore;`,
  },
];

export const getStoreTemplate = (name: string): StoreTemplate | undefined => {
  return STORE_TEMPLATES.find(template => template.name === name);
};

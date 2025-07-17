# How to manage state and contexts

- Use explicit interfaces to define both state and actions
- Keep stores focused on a single need
    - Each store should handle one specific piece of functionality
    - Don't try to manage multiple concerns at once unless the state is shared between multiple components
    - Do not over-engineer solutions to a specific problem, prefer simplicity and elegance
- Group actions near the state properties they modify to make their relationships clear
- Use semantic action names that describe the state change, like `startEditing`or `toggleComplete` over `update` and `set`
- Split state into logical groups (`ui`, `validation`, `data`) to make the structure intuitive and maintainable
- Make state updates atomic and predictable
    - Each action should only update the state it needs to, using immutable patterns
- Include TypeScript interfaces and JSDoc comments that explain the purpose of each part of the store

## Examples

### Counter Store - Basic Pattern

A minimalist store demonstrating best practices for component-specific state management:

#### State Interface Definition

```typescript
/**
 * Core state interface defining all properties that can be read
 * Separates data (count) from UI concerns (animation state)
 */
interface CounterState {
  /** The current numeric value of the counter */
  count: number
  /** UI-specific state properties */
  ui: {
    /** Tracks whether the counter is currently animating */
    isAnimating: boolean
  }
}

/**
 * Actions interface defining all ways the state can be modified
 * Each action represents a single, specific way to update the state
 */
interface CounterActions {
  /** Increases the counter by 1 and triggers animation */
  increment: () => void
  /** Decreases the counter by 1 and triggers animation */
  decrement: () => void
  /** Resets the counter to 0 */
  reset: () => void
  /** Sets a specific value */
  setValue: (value: number) => void
}
```

#### Store Implementation

```typescript
/**
 * Combined interface for the complete store
 */
type CounterStore = CounterState & CounterActions;

/**
 * Create the counter store with Zustand
 */
export const useCounterStore = create<CounterStore>((set, get) => ({
  // Initial state
  count: 0,
  ui: {
    isAnimating: false
  },

  // Actions grouped logically
  increment: () => {
    set((state) => ({
      count: state.count + 1,
      ui: { ...state.ui, isAnimating: true }
    }));
    
    // Reset animation after delay
    setTimeout(() => {
      set((state) => ({
        ...state,
        ui: { ...state.ui, isAnimating: false }
      }));
    }, 200);
  },

  decrement: () => {
    set((state) => ({
      count: Math.max(0, state.count - 1),
      ui: { ...state.ui, isAnimating: true }
    }));
    
    setTimeout(() => {
      set((state) => ({
        ...state,
        ui: { ...state.ui, isAnimating: false }
      }));
    }, 200);
  },

  reset: () => {
    set({ count: 0, ui: { isAnimating: false } });
  },

  setValue: (value: number) => {
    if (value < 0) return; // Validation
    set((state) => ({
      count: value,
      ui: { ...state.ui, isAnimating: false }
    }));
  }
}));
```

#### Component Usage

```typescript
import { useCounterStore } from '../stores/counterStore';

export const Counter = () => {
  // Extract only needed state and actions
  const { count, ui, increment, decrement, reset } = useCounterStore();
  
  return (
    <div className={`transition-transform ${ui.isAnimating ? 'scale-105' : 'scale-100'}`}>
      <span className="text-2xl font-bold">{count}</span>
      <div className="space-x-2">
        <button onClick={decrement}>-</button>
        <button onClick={increment}>+</button>
        <button onClick={reset}>Reset</button>
      </div>
    </div>
  );
};
```

### Todo Store - Complex State Management

A more complex example demonstrating validation, multiple data types, and error handling:

#### State and Types Definition

```typescript
/**
 * Individual todo item interface
 */
interface TodoItem {
  id: string;
  text: string;
  completed: boolean;
  createdAt: Date;
  completedAt?: Date;
  priority: 'low' | 'medium' | 'high';
}

/**
 * Main state interface for the todo store
 */
interface TodoState {
  /** Array of all todo items */
  todos: TodoItem[];
  /** Currently selected filter */
  filter: 'all' | 'active' | 'completed';
  /** Form state for adding new todos */
  form: {
    text: string;
    priority: TodoItem['priority'];
    isValid: boolean;
  };
  /** UI state */
  ui: {
    isLoading: boolean;
    error: string | null;
  };
}

/**
 * Actions interface for todo management
 */
interface TodoActions {
  // Todo CRUD operations
  addTodo: (text: string, priority?: TodoItem['priority']) => void;
  toggleTodo: (id: string) => void;
  deleteTodo: (id: string) => void;
  editTodo: (id: string, text: string) => void;
  
  // Filter operations
  setFilter: (filter: TodoState['filter']) => void;
  
  // Form operations
  updateFormText: (text: string) => void;
  updateFormPriority: (priority: TodoItem['priority']) => void;
  resetForm: () => void;
  
  // Bulk operations
  clearCompleted: () => void;
  toggleAll: () => void;
}
```

#### Store Implementation

```typescript
type TodoStore = TodoState & TodoActions;

export const useTodoStore = create<TodoStore>((set, get) => ({
  // Initial state
  todos: [],
  filter: 'all',
  form: {
    text: '',
    priority: 'medium',
    isValid: false
  },
  ui: {
    isLoading: false,
    error: null
  },

  // Actions
  addTodo: (text: string, priority = 'medium') => {
    const trimmedText = text.trim();
    
    if (!trimmedText) {
      set((state) => ({
        ...state,
        ui: { ...state.ui, error: 'Todo text cannot be empty' }
      }));
      return;
    }

    const newTodo: TodoItem = {
      id: crypto.randomUUID(),
      text: trimmedText,
      completed: false,
      createdAt: new Date(),
      priority
    };

    set((state) => ({
      todos: [...state.todos, newTodo],
      form: { text: '', priority: 'medium', isValid: false },
      ui: { ...state.ui, error: null }
    }));
  },

  toggleTodo: (id: string) => {
    set((state) => ({
      todos: state.todos.map(todo =>
        todo.id === id
          ? {
              ...todo,
              completed: !todo.completed,
              completedAt: !todo.completed ? new Date() : undefined
            }
          : todo
      )
    }));
  },

  deleteTodo: (id: string) => {
    set((state) => ({
      todos: state.todos.filter(todo => todo.id !== id)
    }));
  },

  setFilter: (filter) => {
    set((state) => ({ ...state, filter }));
  },

  updateFormText: (text: string) => {
    set((state) => ({
      form: {
        ...state.form,
        text,
        isValid: text.trim().length > 0
      }
    }));
  },

  clearCompleted: () => {
    set((state) => ({
      todos: state.todos.filter(todo => !todo.completed)
    }));
  }
}));
```

#### Advanced Usage with Selectors

```typescript
// Custom selectors for computed values
export const useTodoSelectors = () => {
  const { todos, filter } = useTodoStore();
  
  return useMemo(() => {
    const filteredTodos = todos.filter(todo => {
      switch (filter) {
        case 'active': return !todo.completed;
        case 'completed': return todo.completed;
        default: return true;
      }
    });
    
    return {
      filteredTodos,
      activeTodosCount: todos.filter(t => !t.completed).length,
      completedTodosCount: todos.filter(t => t.completed).length,
      totalTodosCount: todos.length
    };
  }, [todos, filter]);
};

// Usage in component
export const TodoList = () => {
  const { addTodo, toggleTodo, deleteTodo } = useTodoStore();
  const { filteredTodos, activeTodosCount } = useTodoSelectors();
  
  return (
    <div>
      <p>{activeTodosCount} items left</p>
      {filteredTodos.map(todo => (
        <TodoItem 
          key={todo.id} 
          todo={todo} 
          onToggle={() => toggleTodo(todo.id)}
          onDelete={() => deleteTodo(todo.id)}
        />
      ))}
    </div>
  );
};
```

## Best Practices Summary

### 1. State Structure
- **Separate concerns**: Keep UI state separate from data state
- **Use meaningful names**: State properties should be self-documenting
- **Type everything**: Use TypeScript interfaces for all state and actions

### 2. Action Design
- **Semantic naming**: Use descriptive action names (`startEditing` vs `setMode`)
- **Atomic updates**: Each action should do one specific thing
- **Validation**: Include input validation in actions
- **Error handling**: Set error state for failed operations

### 3. Component Integration
- **Extract selectively**: Only extract the state and actions you need
- **Use selectors**: Create computed values with `useMemo` for complex filtering
- **Handle side effects**: Use `useEffect` for animations and API calls
- **Clean up**: Remove listeners and cancel pending operations

### 4. Performance Tips
- **Avoid object recreation**: Use callbacks to update nested state
- **Memoize selectors**: Use `useMemo` for computed values
- **Split large stores**: Consider separate stores for independent concerns
- **Use shallow equality**: Be mindful of reference equality in React renders

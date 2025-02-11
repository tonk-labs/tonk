# How to manage state and contexts
- Use explicit interfaces to define both state and actions
- Keep stores focused on a single component's needs
    - Each store should handle one specific piece of UI functionality
    - Don't try to manage multiple concerns at once
    - Do not over-engineer solutions to a specific problem, prefer simplicity and elegance
- Group actions near the state properties they modify to make their relationships clear
- Use semantic action names that describe the state change, like `startEditing`or `toggleComplete` over `update` and `set`
- Split state into logical groups (`ui`, `validation`, `data`) to make the structure intuitive and maintainable
- Make state updates atomic and predictable
    - Each action should only update the state it needs to, using immutable patterns
- Include TypeScript interfaces and JSDoc comments that explain the purpose of each part of the store

## Examples
### Counter Store
```tsx
/**
 * @fileoverview A minimalist Zustand store demonstrating best practices for
 * component-specific state management with animations.
 * 
 * Key features:
 * - Atomic state updates
 * - Separation of UI and data concerns
 * - Tight coupling between state and actions
 * - TypeScript type safety
 */

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
  /** Returns counter to 0 and clears animation state */
  reset: () => void
}

/**
 * Combined type for the complete store
 * Merges state and actions to enforce tight coupling
 */
type CounterStore = CounterState & CounterActions

/**
 * Creates a Zustand store for managing counter state and animations
 * 
 * @example
 * const { count, increment } = useCounterStore()
 * return <button onClick={increment}>{count}</button>
 * 
 * @remarks
 * - Each action updates both data and UI state atomically
 * - Animation state is automatically managed with actions
 * - State updates are immutable and predictable
 */
export const useCounterStore = create<CounterStore>((set) => ({
  // Initial state values
  count: 0,
  ui: {
    isAnimating: false
  },

  // Action implementations
  increment: () => set((state) => ({
    count: state.count + 1,
    ui: { isAnimating: true }  // Trigger animation on increment
  })),

  decrement: () => set((state) => ({
    count: state.count - 1,
    ui: { isAnimating: true }  // Trigger animation on decrement
  })),

  reset: () => set({
    count: 0,
    ui: { isAnimating: false }  // Clear animation on reset
  })
}))

/**
 * Example component showing how to consume the counter store
 * 
 * @remarks
 * - Uses selective state extraction for performance
 * - Manages animation cleanup with useEffect
 * - Demonstrates proper store usage patterns
 */
const Counter = () => {
  // Extract only the state and actions needed by this component
  const { count, ui, increment, decrement, reset } = useCounterStore()
  
  // Handle animation timing
  useEffect(() => {
    if (ui.isAnimating) {
      // Clear animation state after delay
      const timer = setTimeout(() => {
        useCounterStore.setState((state) => ({
          ui: { isAnimating: false }
        }))
      }, 300)
      
      // Cleanup timer on unmount or animation state change
      return () => clearTimeout(timer)
    }
  }, [ui.isAnimating])

  return (
    <div className={ui.isAnimating ? 'animate-bounce' : ''}>
      <p>Count: {count}</p>
      <button onClick={increment}>+</button>
      <button onClick={decrement}>-</button>
      <button onClick={reset}>Reset</button>
    </div>
  )
}

/**
 * Usage notes for LLMs:
 * 
 * 1. State Structure:
 *    - Separate data (count) from UI state (isAnimating)
 *    - Keep state minimal and focused
 *    - Use semantic property names
 * 
 * 2. Actions:
 *    - Name actions as commands (increment, reset)
 *    - Update related state together atomically
 *    - Keep actions simple and predictable
 * 
 * 3. Types:
 *    - Define explicit interfaces for state and actions
 *    - Use TypeScript to enforce type safety
 *    - Document types with JSDoc comments
 * 
 * 4. Component Integration:
 *    - Extract only needed state and actions
 *    - Handle side effects (like animation timing) in components
 *    - Use proper cleanup in effects
 */
```

### ToDo Item Store
```tsx
/**
 * Example of an LLM-optimized Zustand store for a TodoItem component
 * Key principles demonstrated:
 * 1. Explicit state interfaces
 * 2. Co-located actions with state
 * 3. Single responsibility
 * 4. Clear type relationships
 * 5. Self-documenting action names
 */

interface TodoItem {
  id: string
  text: string
  isComplete: boolean
  lastModified: Date
}

/**
 * State interface clearly defines all properties that can be accessed
 * Include semantic types rather than primitives where possible
 */
interface TodoItemState {
  // State properties are grouped by related functionality
  item: TodoItem | null
  validation: {
    hasError: boolean
    errorMessage: string
  }
  ui: {
    isEditing: boolean
    isSaving: boolean
  }
}

/**
 * Actions interface defines all ways the state can be modified
 * Each action name describes exactly what it does to the state
 */
interface TodoItemActions {
  // Actions are named as commands that describe state changes
  setItemText: (newText: string) => void
  toggleComplete: () => void
  startEditing: () => void
  finishEditing: () => void
  validateText: (text: string) => boolean
  saveChanges: () => Promise<void>
}

// Combine state and actions into a single store type
type TodoItemStore = TodoItemState & TodoItemActions

/**
 * Create store with co-located state and actions
 * Each action directly updates its related state properties
 */
export const useTodoItemStore = create<TodoItemStore>((set, get) => ({
  // Initial state
  item: null,
  validation: {
    hasError: false,
    errorMessage: ''
  },
  ui: {
    isEditing: false,
    isSaving: false
  },

  // Actions are grouped near the state they modify
  setItemText: (newText) => {
    const isValid = get().validateText(newText)
    set((state) => ({
      item: state.item ? {
        ...state.item,
        text: newText,
        lastModified: new Date()
      } : null,
      validation: {
        hasError: !isValid,
        errorMessage: isValid ? '' : 'Text cannot be empty'
      }
    }))
  },

  toggleComplete: () => 
    set((state) => ({
      item: state.item ? {
        ...state.item,
        isComplete: !state.item.isComplete,
        lastModified: new Date()
      } : null
    })),

  startEditing: () =>
    set((state) => ({
      ui: {
        ...state.ui,
        isEditing: true
      }
    })),

  finishEditing: () =>
    set((state) => ({
      ui: {
        ...state.ui,
        isEditing: false
      }
    })),

  validateText: (text) => text.trim().length > 0,

  saveChanges: async () => {
    set((state) => ({
      ui: {
        ...state.ui,
        isSaving: true
      }
    }))
    
    // API call would go here
    
    set((state) => ({
      ui: {
        ...state.ui,
        isSaving: false,
        isEditing: false
      }
    }))
  }
}))

/**
 * Usage example showing how the store maintains tight coupling
 * between its state and actions while remaining focused on a
 * single component's needs
 */
const TodoItem = ({ id }: { id: string }) => {
  const { 
    item,
    ui,
    validation,
    setItemText,
    toggleComplete,
    startEditing,
    finishEditing,
    saveChanges
  } = useTodoItemStore()

  // Component implementation...
}
```

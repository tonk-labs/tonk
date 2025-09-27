# Tonk Component Creation Guidelines for LLM Agents

## Critical Component Format Requirements

When creating React components in the Tonk environment, you MUST follow these exact patterns.
Components are compiled and hot-reloaded through our bespoke HMR system.

## Component File Structure

### Basic Function Component Pattern

```tsx
import React from 'react';

// Import types and interfaces first
interface ComponentProps {
  prop1: string;
  prop2?: number;
  onAction?: (value: string) => void;
}

// Function component with typed props
const ComponentName: React.FC<ComponentProps> = ({ prop1, prop2 = 0, onAction }) => {
  // State and hooks at the top
  const [state, setState] = React.useState<string>('');

  // Effects after state
  React.useEffect(() => {
    // Effect logic
  }, [dependency]);

  // Event handlers
  const handleClick = () => {
    if (onAction) {
      onAction(state);
    }
  };

  // Render
  return (
    <div className="component-class">
      <h1>{prop1}</h1>
      <button onClick={handleClick}>Click me</button>
    </div>
  );
};

// REQUIRED: Default export
export default ComponentName;
```

## Critical Rules for Components

### 1. Import Requirements

- Always import React explicitly: `import React from 'react';`
- Import hooks from React namespace: `React.useState`, `React.useEffect`
- Can also destructure: `import React, { useState, useEffect } from 'react';`

### 2. Component Definition

- **MUST** use typed props with TypeScript interfaces
- **MUST** use `React.FC<Props>` or function declaration with typed props
- **MUST** use default export
- Component name should be PascalCase

### 3. Hooks Rules

- All hooks **MUST** be at the top level of the component
- Never call hooks inside conditions, loops, or nested functions
- Order: useState, useReducer, useContext, useEffect, custom hooks

### 4. File Naming

- Component files: `/src/components/ComponentName.tsx`
- Use PascalCase for component filenames
- Match filename to component name

### 5. Store Integration

When using Tonk stores in components:

```tsx
import React from 'react';
import useExampleStore from '../stores/example-store';

interface MyComponentProps {
  title: string;
}

const MyComponent: React.FC<MyComponentProps> = ({ title }) => {
  // Access store state and actions
  const { data, isLoading, fetchData } = useExampleStore();

  React.useEffect(() => {
    fetchData();
  }, []);

  if (isLoading) {
    return <div>Loading...</div>;
  }

  return (
    <div>
      <h1>{title}</h1>
      <div>{data}</div>
    </div>
  );
};

export default MyComponent;
```

## Common Component Patterns

### Form Component with State

```tsx
import React from 'react';

interface FormData {
  name: string;
  email: string;
  message: string;
}

interface ContactFormProps {
  onSubmit: (data: FormData) => void;
}

const ContactForm: React.FC<ContactFormProps> = ({ onSubmit }) => {
  const [formData, setFormData] = React.useState<FormData>({
    name: '',
    email: '',
    message: '',
  });

  const [errors, setErrors] = React.useState<Partial<FormData>>({});

  const handleChange = (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    const { name, value } = e.target;
    setFormData(prev => ({
      ...prev,
      [name]: value,
    }));
    // Clear error when user starts typing
    if (errors[name as keyof FormData]) {
      setErrors(prev => ({
        ...prev,
        [name]: undefined,
      }));
    }
  };

  const validateForm = (): boolean => {
    const newErrors: Partial<FormData> = {};

    if (!formData.name.trim()) {
      newErrors.name = 'Name is required';
    }

    if (!formData.email.trim()) {
      newErrors.email = 'Email is required';
    } else if (!/\S+@\S+\.\S+/.test(formData.email)) {
      newErrors.email = 'Email is invalid';
    }

    if (!formData.message.trim()) {
      newErrors.message = 'Message is required';
    }

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();

    if (validateForm()) {
      onSubmit(formData);
      // Reset form
      setFormData({ name: '', email: '', message: '' });
    }
  };

  return (
    <form onSubmit={handleSubmit}>
      <div>
        <input
          type="text"
          name="name"
          value={formData.name}
          onChange={handleChange}
          placeholder="Name"
        />
        {errors.name && <span className="error">{errors.name}</span>}
      </div>

      <div>
        <input
          type="email"
          name="email"
          value={formData.email}
          onChange={handleChange}
          placeholder="Email"
        />
        {errors.email && <span className="error">{errors.email}</span>}
      </div>

      <div>
        <textarea
          name="message"
          value={formData.message}
          onChange={handleChange}
          placeholder="Message"
        />
        {errors.message && <span className="error">{errors.message}</span>}
      </div>

      <button type="submit">Submit</button>
    </form>
  );
};

export default ContactForm;
```

### List Component with Filtering

```tsx
import React from 'react';

interface Item {
  id: string;
  name: string;
  category: string;
  price: number;
}

interface ItemListProps {
  items: Item[];
  onItemClick?: (item: Item) => void;
}

const ItemList: React.FC<ItemListProps> = ({ items, onItemClick }) => {
  const [filter, setFilter] = React.useState<string>('');
  const [sortBy, setSortBy] = React.useState<'name' | 'price'>('name');

  const filteredItems = React.useMemo(() => {
    let filtered = items.filter(
      item =>
        item.name.toLowerCase().includes(filter.toLowerCase()) ||
        item.category.toLowerCase().includes(filter.toLowerCase())
    );

    return filtered.sort((a, b) => {
      if (sortBy === 'name') {
        return a.name.localeCompare(b.name);
      }
      return a.price - b.price;
    });
  }, [items, filter, sortBy]);

  return (
    <div className="item-list">
      <div className="controls">
        <input
          type="text"
          value={filter}
          onChange={e => setFilter(e.target.value)}
          placeholder="Filter items..."
        />

        <select value={sortBy} onChange={e => setSortBy(e.target.value as 'name' | 'price')}>
          <option value="name">Sort by Name</option>
          <option value="price">Sort by Price</option>
        </select>
      </div>

      <div className="items">
        {filteredItems.map(item => (
          <div key={item.id} className="item" onClick={() => onItemClick?.(item)}>
            <h3>{item.name}</h3>
            <p>{item.category}</p>
            <span>${item.price.toFixed(2)}</span>
          </div>
        ))}
      </div>

      {filteredItems.length === 0 && <p>No items found matching your criteria.</p>}
    </div>
  );
};

export default ItemList;
```

### Component with Async Data Loading

```tsx
import React from 'react';

interface User {
  id: string;
  name: string;
  email: string;
}

interface UserProfileProps {
  userId: string;
}

const UserProfile: React.FC<UserProfileProps> = ({ userId }) => {
  const [user, setUser] = React.useState<User | null>(null);
  const [loading, setLoading] = React.useState<boolean>(true);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;

    const fetchUser = async () => {
      setLoading(true);
      setError(null);

      try {
        const response = await fetch(`/api/users/${userId}`);
        if (!response.ok) {
          throw new Error('Failed to fetch user');
        }

        const data = await response.json();

        if (!cancelled) {
          setUser(data);
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : 'An error occurred');
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    };

    fetchUser();

    // Cleanup function
    return () => {
      cancelled = true;
    };
  }, [userId]);

  if (loading) {
    return <div className="loading">Loading user profile...</div>;
  }

  if (error) {
    return <div className="error">Error: {error}</div>;
  }

  if (!user) {
    return <div>User not found</div>;
  }

  return (
    <div className="user-profile">
      <h2>{user.name}</h2>
      <p>Email: {user.email}</p>
      <p>ID: {user.id}</p>
    </div>
  );
};

export default UserProfile;
```

## Event Handling Best Practices

```tsx
import React from 'react';

interface ButtonGroupProps {
  onAction: (action: string, value: any) => void;
}

const ButtonGroup: React.FC<ButtonGroupProps> = ({ onAction }) => {
  // Inline arrow function (simple cases)
  const handleSimpleClick = () => {
    onAction('simple', null);
  };

  // Event parameter handling
  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    onAction('change', e.target.value);
  };

  // Parameterized handler using closure
  const createHandler = (action: string) => () => {
    onAction(action, Date.now());
  };

  // Prevent default and stop propagation
  const handleFormSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    e.stopPropagation();
    onAction('submit', new FormData(e.target as HTMLFormElement));
  };

  return (
    <div>
      <button onClick={handleSimpleClick}>Simple</button>
      <button onClick={createHandler('action1')}>Action 1</button>
      <button onClick={createHandler('action2')}>Action 2</button>
      <input onChange={handleInputChange} />
      <form onSubmit={handleFormSubmit}>
        <button type="submit">Submit</button>
      </form>
    </div>
  );
};

export default ButtonGroup;
```

## Performance Optimization Patterns

```tsx
import React from 'react';

interface ExpensiveComponentProps {
  data: any[];
  filter: string;
}

const ExpensiveComponent: React.FC<ExpensiveComponentProps> = ({ data, filter }) => {
  // Memoize expensive computations
  const processedData = React.useMemo(() => {
    console.log('Processing data...');
    return data
      .filter(item => item.name.includes(filter))
      .map(item => ({
        ...item,
        computed: heavyComputation(item),
      }));
  }, [data, filter]);

  // Memoize callbacks to prevent unnecessary re-renders
  const handleClick = React.useCallback((id: string) => {
    console.log('Clicked:', id);
  }, []);

  // Use refs for values that don't trigger re-renders
  const renderCount = React.useRef(0);
  renderCount.current++;

  return (
    <div>
      <p>Render count: {renderCount.current}</p>
      {processedData.map(item => (
        <div key={item.id} onClick={() => handleClick(item.id)}>
          {item.name}
        </div>
      ))}
    </div>
  );
};

// Helper function
function heavyComputation(item: any): any {
  // Simulate expensive operation
  return item;
}

export default ExpensiveComponent;
```

## Common Mistakes to Avoid

1. ❌ **DON'T** forget to add key props to list items
2. ❌ **DON'T** mutate state directly - always create new objects/arrays
3. ❌ **DON'T** use array index as key if list items can be reordered
4. ❌ **DON'T** forget cleanup in useEffect when necessary
5. ❌ **DON'T** call hooks conditionally or in loops
6. ❌ **DON'T** forget to handle loading and error states for async operations
7. ❌ **DON'T** use named exports for components - always use default export

## TypeScript Best Practices

1. ✅ Always define interfaces for props
2. ✅ Use proper event types: `React.ChangeEvent<HTMLInputElement>`
3. ✅ Type useState properly: `useState<string>('')`
4. ✅ Use `React.FC<Props>` for functional components
5. ✅ Avoid using `any` - use proper types or `unknown` if type is truly unknown
6. ✅ Use optional chaining for optional props: `onAction?.('value')`

## File Validation Rules

Components will be validated for:

- Proper TypeScript syntax
- React hooks rules
- No unused variables
- Proper imports
- Default export requirement
- Semicolons at statement ends
- 2-space indentation

## Integration with Tonk Stores

When components use Tonk stores:

1. Import the store using its default export
2. Call the store hook at the top level of the component
3. Destructure only the state and actions you need
4. Remember that stores handle their own persistence via JSON files

```tsx
import React from 'react';
import useTodoStore from '../stores/todo-store';

const TodoList: React.FC = () => {
  const { todos, addTodo, toggleTodo, filter, setFilter } = useTodoStore();
  const [inputValue, setInputValue] = React.useState('');

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (inputValue.trim()) {
      addTodo(inputValue);
      setInputValue('');
    }
  };

  return (
    <div>
      <form onSubmit={handleSubmit}>
        <input
          value={inputValue}
          onChange={e => setInputValue(e.target.value)}
          placeholder="Add todo..."
        />
        <button type="submit">Add</button>
      </form>

      <div>
        <button onClick={() => setFilter('all')}>All</button>
        <button onClick={() => setFilter('active')}>Active</button>
        <button onClick={() => setFilter('completed')}>Completed</button>
      </div>

      <ul>
        {todos.map(todo => (
          <li key={todo.id} onClick={() => toggleTodo(todo.id)}>
            <span style={{ textDecoration: todo.completed ? 'line-through' : 'none' }}>
              {todo.text}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
};

export default TodoList;
```

Remember: Components in Tonk are hot-reloaded automatically when saved. The validation system will
catch errors before they break the application.

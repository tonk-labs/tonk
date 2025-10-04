# Tonk Development Manual

## Overview

Tonk is a dynamic development environment where components, pages, and stores are stored in a
virtual file system (VFS) and compiled at runtime. All code is written in TypeScript without
imports - libraries are globally available.

## Critical Rules - READ FIRST

### NO IMPORTS - Everything is Global

**NEVER use import or require statements.** All libraries and dependencies are globally available in
the compilation context.

```tsx
// ❌ WRONG - No imports allowed
import React from 'react';
import { useState } from 'react';

// ✅ CORRECT - Use directly
const MyComponent = () => {
  const [count, setCount] = useState(0);
  return <div>{count}</div>;
};
```

### Available Global Libraries

**React & Hooks:**

- `React`, `useState`, `useEffect`, `useCallback`, `useMemo`, `useRef`, `useReducer`, `useContext`,
  `Fragment`

**React Router:**

- `Link`, `NavLink`, `useNavigate`, `useLocation`, `useParams`, `useSearchParams`

**Zustand (for stores):**

- `create`, `sync`

**All registered components and stores are globally available after compilation.**

---

## Components

### File Location & Requirements

- **Location:** `/src/components/ComponentName.tsx`
- **Extension:** Must use `.tsx`
- **Export:** Must use default export
- **Typing:** Must use TypeScript with proper interfaces

### Basic Component Pattern

```tsx
interface ComponentProps {
  title: string;
  count?: number;
  onAction?: () => void;
}

const MyComponent: React.FC<ComponentProps> = ({ title, count = 0, onAction }) => {
  const [isActive, setIsActive] = useState(false);

  useEffect(() => {
    console.log('Component mounted');
  }, []);

  const handleClick = () => {
    setIsActive(!isActive);
    onAction?.();
  };

  return (
    <div className="p-6 bg-white rounded-lg shadow-md">
      <h1 className="text-2xl font-bold mb-4">{title}</h1>
      <p className="mb-4">Count: {count}</p>
      <button
        className={`px-4 py-2 rounded ${isActive ? 'bg-green-500' : 'bg-blue-500'} text-white`}
        onClick={handleClick}
      >
        {isActive ? 'Active' : 'Inactive'}
      </button>
    </div>
  );
};

export default MyComponent;
```

### Store Integration in Components

**CRITICAL: Prevent Input Focus Loss**

When a parent component displays a child with form inputs, **DO NOT** subscribe to the entire store
state or arrays that change frequently. This causes the child to re-render and inputs lose focus.

```tsx
// ❌ BAD - Causes input focus loss
const ContactPage: React.FC = () => {
  const { submissions } = useContactStore(); // Re-renders on every change!

  return (
    <div>
      <ContactForm /> {/* Will lose focus when submissions update */}
      <p>Total: {submissions.length}</p>
    </div>
  );
};

// ✅ GOOD - Selective subscription
const ContactPage: React.FC = () => {
  const count = useContactStore(state => state.submissions.length);

  return (
    <div>
      <ContactForm /> {/* Won't re-render unnecessarily */}
      <p>Total: {count}</p>
    </div>
  );
};
```

**Rules for Store Subscriptions:**

1. Never subscribe to arrays/objects if you only need derived values
2. Use selectors to extract only what you need
3. Parent components with forms MUST use minimal subscriptions
4. Subscribe only to counts, booleans, or primitive values

### Using Stores in Components

```tsx
const MyComponent: React.FC = () => {
  const { data, isLoading, fetchData } = useExampleStore();

  useEffect(() => {
    fetchData();
  }, []);

  if (isLoading) return <div>Loading...</div>;

  return (
    <div className="p-4">
      <p>{data}</p>
    </div>
  );
};
```

### Navigation Rules

**ALWAYS use absolute paths starting with `/`:**

```tsx
// ✅ CORRECT
<Link to="/about">About</Link>
<Link to="/products">Products</Link>
navigate('/dashboard');

// ❌ WRONG - Never use relative paths
<Link to="about">About</Link>
navigate('products');
```

**Use Link component, NOT `<a>` tags:**

```tsx
// ✅ CORRECT
<Link to="/about" className="text-blue-500">About Us</Link>

// ❌ WRONG
<a href="/about">About Us</a>
```

**Programmatic navigation:**

```tsx
const MyComponent: React.FC = () => {
  const navigate = useNavigate();

  const handleClick = () => {
    navigate('/dashboard');
    // or with options:
    navigate('/profile', { replace: true, state: { from: 'home' } });
  };

  return <button onClick={handleClick}>Go to Dashboard</button>;
};
```

### Component Checklist

- [ ] **NO imports** - All libraries used directly
- [ ] **Default export** - Component must export default
- [ ] **TypeScript props** - Interface defined for props
- [ ] **Selective store subscriptions** - Parent components use selectors
- [ ] **Absolute navigation paths** - All routes start with `/`
- [ ] **Managed form inputs** - If using forms with stores

---

## Pages (Views)

### File Location & Naming

- **Location:** `/src/views/` directory
- **Extension:** Must use `.tsx`
- **Route mapping:**
  - `/src/views/index.tsx` → `/` (homepage) **← THE MOST IMPORTANT FILE**
  - `/src/views/about.tsx` → `/about`
  - `/src/views/products.tsx` → `/products`
  - `/src/views/admin/dashboard.tsx` → `/admin/dashboard`

**IMPORTANT:** `index.tsx` is the homepage - don't create `homepage.tsx` or `Index.tsx`.

### THE HOME PAGE - CRITICAL WORKFLOW

**THE MOST IMPORTANT FILE: `/src/views/index.tsx`**

This is the MAIN HOME PAGE that users see when viewing the app (not in edit mode).

**When a user asks you to "make something" or "create something", follow this workflow:**

1. **FIRST**: Check if `/src/views/index.tsx` exists
2. **READ**: If it exists, read its content to understand what's there

3. **THEN** choose your path:

   **Path A - No index.tsx exists:**

   - Create the component in `/src/components/`
   - CREATE `/src/views/index.tsx` and add the component to it
   - Tell user: "I created [Component] and added it to your home page"

   **Path B - index.tsx exists but is empty/minimal:**

   - Create the component in `/src/components/`
   - UPDATE `/src/views/index.tsx` to include the component
   - Tell user: "I created [Component] and added it to your home page"

   **Path C - index.tsx has real content:**

   - Create ONLY the component in `/src/components/`
   - Tell user: "I created the [Component] component. Would you like me to add it to your home page,
     or would you prefer to add it to a different page?"
   - WAIT for user response before modifying index.tsx

### Page Component Structure

```typescript
const YourPageName: React.FC = () => {
  const [state, setState] = useState<string>('');

  useEffect(() => {
    // Side effects here
  }, []);

  const handleAction = () => {
    // Handler logic
  };

  return (
    <div className="container mx-auto p-6">
      <h1 className="text-3xl font-bold mb-6">Page Title</h1>
      {/* Page content */}
    </div>
  );
};

export default YourPageName;
```

### Styling with Tailwind CSS

```typescript
<div className="container mx-auto px-4 py-8">
  <h1 className="text-4xl font-bold text-gray-900 mb-6">
    Welcome
  </h1>

  <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
    <div className="bg-white rounded-lg shadow-md p-6 hover:shadow-lg transition-shadow">
      <h2 className="text-xl font-semibold mb-3">Card Title</h2>
      <p className="text-gray-600">Card content</p>
    </div>
  </div>
</div>
```

### MOBILE FIRST Design

```typescript
<nav className="flex flex-col sm:flex-row items-center justify-between p-4">
  <div className="mb-4 sm:mb-0">Logo</div>
  <div className="grid grid-cols-2 sm:flex sm:space-x-4">
    {/* Navigation items */}
  </div>
</nav>
```

---

## Stores

### File Location & Requirements

- **Location:** `/src/stores/store-name.ts`
- **Extension:** Must use `.ts` (NOT `.tsx`)
- **Export:** Must use default export
- **Persistence:** State persisted to JSON file at same path

### Mandatory Store Structure

```typescript
// REQUIRED: This exact comment must appear at the top
// create and sync are available in the compilation context

// 1. Define TypeScript interfaces
interface YourStateInterface {
  // State properties
  property1: type;
  property2: type;

  // Action methods
  actionMethod: (param: type) => void;
  asyncMethod: (param: type) => Promise<void>;
  getterMethod: () => type;
}

// 2. Create the store using this EXACT pattern
const useYourStore = create<YourStateInterface>()(
  sync(
    (set, get) => ({
      // Initial state values
      property1: initialValue,
      property2: initialValue,

      // Action implementations
      actionMethod: param => {
        set(state => ({
          property1: newValue,
        }));
      },

      // Async actions allowed
      asyncMethod: async param => {
        const result = await someAsyncOperation();
        set({ property1: result });
      },

      // Can use get() to access current state
      getterMethod: () => {
        const currentState = get();
        return currentState.property1;
      },
    }),
    // REQUIRED: sync configuration with path
    { path: '/src/stores/your-store-name.json' }
  )
);

// REQUIRED: Default export
export default useYourStore;
```

### Critical Store Rules

1. **Import Comment:** Must have `// create and sync are available in the compilation context` as
   first line
2. **Store Pattern:** Must use `create<Interface>()()` with double parentheses
3. **Sync Wrapper:** Must wrap with `sync()`
4. **JSON Path:** Must provide `{ path: '/src/stores/store-name.json' }` - this enables state
   persistence
5. **State Updates:** Use `set()` immutably - spread existing objects/arrays when modifying
6. **Export:** Must use default export

### Common Store Patterns

**ID Generation:**

```typescript
const id = Date.now().toString() + Math.random().toString(36).substr(2, 9);
```

**Array Updates:**

```typescript
// Adding items
set(state => ({
  items: [...state.items, newItem],
}));

// Removing items
set(state => ({
  items: state.items.filter(item => item.id !== idToRemove),
}));

// Updating items
set(state => ({
  items: state.items.map(item => (item.id === idToUpdate ? { ...item, ...updates } : item)),
}));
```

**Async Actions:**

```typescript
asyncAction: async params => {
  set({ isLoading: true });

  try {
    const result = await someAsyncCall(params);
    set({
      data: result,
      isLoading: false,
      error: null,
    });
  } catch (error) {
    set({
      isLoading: false,
      error: error.message,
    });
  }
};
```

**Using get() for Current State:**

```typescript
actionWithCurrentState: () => {
  const { property1, property2 } = get();
  if (property1 > 10) {
    set({ property2: 'high' });
  }
};
```

### Store Mistakes to Avoid

1. ❌ **DON'T** import create or sync - they're globally available
2. ❌ **DON'T** forget the sync wrapper
3. ❌ **DON'T** forget the path configuration
4. ❌ **DON'T** use named exports - always use default export
5. ❌ **DON'T** mutate state directly - always create new objects/arrays
6. ❌ **DON'T** forget the comment at the top
7. ❌ **DON'T** use `.tsx` extension - stores are `.ts` files only

---

## Common Error Patterns & Solutions

### Input Focus Loss

**Symptom:** Cursor jumps or loses focus when typing in form fields.

**Cause:** Parent component subscribing to frequently-changing store state.

```tsx
// ❌ WRONG - Parent subscribes to entire array
const ContactPage: React.FC = () => {
  const { submissions } = useContactStore(); // Re-renders every time!

  return (
    <div>
      <ContactForm /> {/* Loses focus on parent re-render */}
      <p>Total: {submissions.length}</p>
    </div>
  );
};

// ✅ CORRECT - Selective subscription
const ContactPage: React.FC = () => {
  const count = useContactStore(state => state.submissions.length);

  return (
    <div>
      <ContactForm /> {/* Won't re-render unnecessarily */}
      <p>Total: {count}</p>
    </div>
  );
};
```

### Navigation Not Working

**Symptom:** Links don't work or cause page reload.

```tsx
// ❌ WRONG - Relative path
<Link to="about">About</Link>
navigate('products');

// ✅ CORRECT - Absolute path starting with /
<Link to="/about">About</Link>
navigate('/products');

// ❌ WRONG - Using <a> tag
<a href="/about">About</a>

// ✅ CORRECT - Using Link component
<Link to="/about">About</Link>
```

**Rules:**

- ALWAYS use absolute paths: `/about` not `about`
- ALWAYS use `Link` component, never `<a>`
- Use `useNavigate()` for programmatic navigation

### Store/Component Not Found

**Symptom:** "useExampleStore is not defined" or similar.

```tsx
// ❌ WRONG - Trying to import
import { useExampleStore } from '../stores/example';

// ✅ CORRECT - No import, globally available
const MyComponent = () => {
  const { data } = useExampleStore();
  return <p>{data}</p>;
};
```

### Forms Not Updating Store

**Symptom:** Typing in inputs but store doesn't update.

```tsx
// ❌ WRONG - Not controlled
<input placeholder="Name" />;

// ✅ CORRECT - Controlled input with store
const FormComponent = () => {
  const name = useFormStore(state => state.name);
  const setName = useFormStore(state => state.setName);

  return <input value={name} onChange={e => setName(e.target.value)} placeholder="Name" />;
};
```

**Rule:** All form inputs must be controlled (value + onChange) and connected to store state.

### Import/Require Errors

**Symptom:** "Cannot find module" or "import is not defined"

```tsx
// ❌ WRONG - No imports allowed
import React from 'react';
const something = require('./utils');

// ✅ CORRECT - No imports needed
const MyComponent = () => {
  return <div>Everything is global</div>;
};

export default MyComponent;
```

### TypeScript Errors

**Props not typed:**

```tsx
// ❌ WRONG
const MyComponent = ({ title, count }) => { ... }

// ✅ CORRECT
interface MyComponentProps {
  title: string;
  count?: number;
}
const MyComponent: React.FC<MyComponentProps> = ({ title, count = 0 }) => { ... }
```

**Event handlers not typed:**

```tsx
// ❌ WRONG
const handleChange = (e) => { ... }

// ✅ CORRECT
const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => { ... }
```

---

## Architecture Best Practices

### Component Size & Organization

- Keep components below 300 lines of code
- Extract reusable UI patterns into separate components
- Create base UI components (Button, Input, Select) for consistency
- Always reuse existing components for similar patterns

### Development Approach

- ALWAYS develop MOBILE FIRST
- Use Tailwind CSS responsive utilities (sm:, md:, lg:, xl:)
- Create component libraries for common UI patterns
- Keep business logic in stores, not components
- Use TypeScript strictly - no `any` types

### File Organization

```
/src/components/    - Reusable UI components
/src/views/         - Page views (route-mapped)
/src/stores/        - Zustand state stores
```

---

## Quick Error Checklist

When you get an error, check:

1. **Did you use any imports?** → Remove them all
2. **Is a parent component causing re-renders?** → Use selective store subscriptions
3. **Are paths absolute?** → Use `/about` not `about`
4. **Are inputs controlled?** → Connect value and onChange to store
5. **Did you export default?** → Component/store must export default
6. **Using `.tsx` for stores?** → Stores must be `.ts` files
7. **Forgot sync path?** → Stores need `{ path: '/src/stores/name.json' }`

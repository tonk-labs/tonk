# Tonk Component Creation Guidelines for LLM Agents

## Critical Component Format Requirements

When creating React components in the Tonk environment, you MUST follow these exact patterns.
Components are compiled and hot-reloaded through our bespoke HMR system.

## Component File Structure

### Basic Function Component Pattern

```tsx
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
    <div className="p-4 bg-white rounded-lg shadow-md">
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

When using Tonk stores in components, NEVER import the store - it is hotlinked

```tsx
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
    <div className="p-4 bg-white rounded-lg shadow-md">
      <h1 className="text-2xl font-bold mb-4">{title}</h1>
      <div>{data}</div>
    </div>
  );
};

export default MyComponent;
```

### 6. Navigation with React Router

For navigation between pages, use React Router APIs that are available in the Tonk environment:

```tsx
const NavigationComponent: React.FC = () => {
  const navigate = useNavigate();
  const location = useLocation();
  const params = useParams();

  const handleNavigation = () => {
    // Programmatic navigation - MUST use absolute paths starting with '/'
    navigate('/about');
    navigate('/products', { replace: true });
  };

  return (
    <div>
      {/* Link navigation - MUST use absolute paths starting with '/' */}
      <Link to="/about" className="text-blue-500 hover:underline">
        About Us
      </Link>

      <Link to="/products" className="px-4 py-2 bg-blue-500 text-white rounded">
        View Products
      </Link>

      {/* NavLink for active state styling */}
      <NavLink
        to="/dashboard"
        className={({ isActive }) => (isActive ? 'text-blue-600 font-bold' : 'text-gray-600')}
      >
        Dashboard
      </NavLink>

      <button onClick={handleNavigation}>Go to About</button>
    </div>
  );
};
```

**CRITICAL Navigation Rules:**

- ✅ **ALWAYS** use absolute paths starting with `/` (e.g., `/about`, `/products`)
- ✅ **NEVER** use relative paths without `/` (e.g., `about`, `products`)
- ✅ **USE** `<Link>` for internal navigation instead of `<a>` tags
- ✅ **USE** `useNavigate()` for programmatic navigation
- ❌ **NEVER** use `window.location` for navigation
- ❌ **NEVER** use `<a href="/path">` for internal links

# IMPORTANT

## Forms and Inputs

When you create input of forms that use a store- the input boxes MUST be managed or otherwise the
input boxes will constantly reset after typing and the users can't use your forms.

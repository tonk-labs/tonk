# Tonk Page Creation Guidelines for LLM Agents

## Overview

Pages in Tonk are React components stored in `/src/views/` that are dynamically compiled and
rendered at runtime. The routing system automatically maps URL paths to view files, enabling hot
module replacement and live editing.

## Critical Page Structure Requirements

### 1. File Location & Naming

- **MUST** be placed in `/src/views/` directory
- File name determines the route:
  - `/src/views/Index.tsx` → `/` (homepage)
  - `/src/views/About.tsx` → `/about`
  - `/src/views/Products.tsx` → `/products`
  - `/src/views/admin/Dashboard.tsx` → `/admin/dashboard`
- **MUST** use `.tsx` extension (TypeScript + JSX)

### 2. Page Component Structure

!! IMPORTANT: IMPORTS HAPPEN AUTOMATIGICALLY, only write the components

```typescript
// Define the page component
const YourPageName: React.FC = () => {
  // Use hooks at the top level
  const [state, setState] = useState<string>('');

  // Define handlers and effects
  useEffect(() => {
    // Side effects here
  }, []);

  const handleAction = () => {
    // Handler logic
  };

  // Return JSX
  return (
    <div className="container mx-auto p-6">
      <h1 className="text-3xl font-bold mb-6">Page Title</h1>
      {/* Page content */}
    </div>
  );
};

// CRITICAL: Must use default export
export default YourPageName;
```

## Available Packages and Imports

Pages have access to these pre-loaded packages (no need to install):

### Core React

```typescript
import React, { useState, useEffect, useCallback, useMemo, useRef } from 'react';
```

## Routing System

### How Routes Work

1. The `App.tsx` component maps routes to view files automatically
2. URL path directly corresponds to file path in `/src/views/`

### Navigation Patterns

```typescript
// Link navigation
<Link to="/about">About Us</Link> // goes to /src/views/about.tsx
<Link to="/products" className="text-blue-500 hover:underline">
  View Products
</Link>
```

## Styling Guidelines

### Using Tailwind CSS

Pages should use Tailwind CSS classes for styling:

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

## State Management in Pages

### Local State

```typescript
const [isOpen, setIsOpen] = useState(false);
const [formData, setFormData] = useState({
  name: '',
  email: '',
});
```

### Store Integration

```typescript
const TodoPage: React.FC = () => {
  // Access store state and actions
  const { todos, addTodo, toggleTodo, filter, setFilter } = useTodoStore();
  const [newTodoText, setNewTodoText] = useState('');

  const handleAddTodo = () => {
    if (newTodoText.trim()) {
      addTodo(newTodoText);
      setNewTodoText('');
    }
  };

  return (
    <div>
      {/* Use store data */}
      {todos.map(todo => (
        <div key={todo.id} onClick={() => toggleTodo(todo.id)}>
          {todo.text}
        </div>
      ))}
    </div>
  );
};
```

## Critical Architecture Patterns

- ALWAYS keep components below <300 LOC
- IF UI COMPONENTS DON'T EXIST, create them as components
  - Button
  - Input
  - Select
- ALWAYS create UI components for style and re-use
- ALWAYS RE-USE existing components for patterns
- ALWAYS develop MOBILE FIRST

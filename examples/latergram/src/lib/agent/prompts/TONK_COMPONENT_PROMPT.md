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

# IMPORTANT

## Forms and Inputs

When you create input of forms that use a store- the input boxes MUST be managed or otherwise the
input boxes will constantly reset after typing and the users can't use your forms.

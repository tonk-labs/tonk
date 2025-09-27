# Tonk Page Creation Guidelines for LLM Agents

## Overview

Pages in Tonk are React components stored in `/src/views/` that are dynamically compiled and
rendered at runtime. The routing system automatically maps URL paths to view files, enabling hot
module replacement and live editing.

## Critical Page Structure Requirements

### 1. File Location & Naming

- **MUST** be placed in `/src/views/` directory
- File name determines the route:
  - `/src/views/index.tsx` → `/` (homepage)
  - `/src/views/about.tsx` → `/about`
  - `/src/views/products.tsx` → `/products`
  - `/src/views/admin/dashboard.tsx` → `/admin/dashboard`
- **MUST** use `.tsx` extension (TypeScript + JSX)

### 2. Page Component Structure

```typescript
import React from 'react';
// Import hooks, components, and utilities from available packages
import { useState, useEffect } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { Button } from '../components/ui/button'; // Example component import

// Import any stores you need
import useAuthStore from '../stores/auth-store';
import useThemeStore from '../stores/theme-store';

// Define the page component
const YourPageName: React.FC = () => {
  // Use hooks at the top level
  const [state, setState] = useState<string>('');
  const navigate = useNavigate();
  const params = useParams();

  // Access stores
  const { user, isAuthenticated } = useAuthStore();
  const { theme } = useThemeStore();

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

### Routing (React Router v6)

```typescript
import {
  Link,
  useNavigate,
  useParams,
  useLocation,
  useSearchParams,
  Routes,
  Route,
  Navigate,
} from 'react-router-dom';
```

### Icons (Lucide React)

```typescript
import {
  Home,
  Settings,
  User,
  Search,
  Menu,
  X,
  ChevronRight,
  // ... hundreds more icons available
} from 'lucide-react';
```

### State Management (Zustand stores)

```typescript
// Import stores from /src/stores/
import useCounterStore from '../stores/counter-store';
import useAuthStore from '../stores/auth-store';
// etc.
```

### Component Libraries (if available in project)

```typescript
// Import from components directory
import { Button } from '../components/ui/button';
import { Card } from '../components/ui/card';
import Header from '../components/Header';
```

## Routing System

### How Routes Work

1. The `App.tsx` component maps routes to view files automatically
2. URL path directly corresponds to file path in `/src/views/`
3. Dynamic routing is handled via `useParams()`

### Route Examples

```typescript
// Static route: /src/views/dashboard.tsx
const Dashboard: React.FC = () => {
  return <div>Dashboard</div>;
};

// Dynamic route: /src/views/user/[id].tsx (if supported)
// Or use catch-all with useParams
const UserProfile: React.FC = () => {
  const { id } = useParams<{ id: string }>();
  return <div>User {id}</div>;
};
```

### Navigation Patterns

```typescript
// Link navigation
<Link to="/about">About Us</Link>
<Link to="/products" className="text-blue-500 hover:underline">
  View Products
</Link>

// Programmatic navigation
const navigate = useNavigate();

const handleLogin = async () => {
  // ... login logic
  navigate('/dashboard');
};

// With state
navigate('/checkout', { state: { cartItems } });

// Go back
navigate(-1);
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

### Responsive Design

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

## Common Page Patterns

### 1. Dashboard Page

```typescript
import React from 'react';
import { BarChart, Users, DollarSign, Activity } from 'lucide-react';

const Dashboard: React.FC = () => {
  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white shadow">
        <div className="max-w-7xl mx-auto py-6 px-4">
          <h1 className="text-3xl font-bold text-gray-900">Dashboard</h1>
        </div>
      </header>

      <main>
        <div className="max-w-7xl mx-auto py-6 sm:px-6 lg:px-8">
          <div className="grid grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-4">
            {/* Stat cards */}
            <div className="bg-white overflow-hidden shadow rounded-lg">
              <div className="p-5">
                <div className="flex items-center">
                  <div className="flex-shrink-0">
                    <Users className="h-6 w-6 text-gray-400" />
                  </div>
                  <div className="ml-5 w-0 flex-1">
                    <dl>
                      <dt className="text-sm font-medium text-gray-500 truncate">
                        Total Users
                      </dt>
                      <dd className="text-lg font-semibold text-gray-900">
                        1,234
                      </dd>
                    </dl>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </main>
    </div>
  );
};

export default Dashboard;
```

### 2. Form Page

```typescript
import React, { useState } from 'react';
import { useNavigate } from 'react-router-dom';

const ContactForm: React.FC = () => {
  const navigate = useNavigate();
  const [formData, setFormData] = useState({
    name: '',
    email: '',
    message: ''
  });

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    // Handle form submission
    console.log('Form submitted:', formData);
    navigate('/thank-you');
  };

  const handleChange = (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    setFormData(prev => ({
      ...prev,
      [e.target.name]: e.target.value
    }));
  };

  return (
    <div className="max-w-md mx-auto mt-8 p-6 bg-white rounded-lg shadow">
      <h2 className="text-2xl font-bold mb-6">Contact Us</h2>

      <form onSubmit={handleSubmit} className="space-y-4">
        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1">
            Name
          </label>
          <input
            type="text"
            name="name"
            value={formData.name}
            onChange={handleChange}
            className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
            required
          />
        </div>

        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1">
            Email
          </label>
          <input
            type="email"
            name="email"
            value={formData.email}
            onChange={handleChange}
            className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
            required
          />
        </div>

        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1">
            Message
          </label>
          <textarea
            name="message"
            value={formData.message}
            onChange={handleChange}
            rows={4}
            className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
            required
          />
        </div>

        <button
          type="submit"
          className="w-full bg-blue-500 text-white py-2 px-4 rounded-md hover:bg-blue-600 transition-colors"
        >
          Send Message
        </button>
      </form>
    </div>
  );
};

export default ContactForm;
```

### 3. List Page with Filtering

```typescript
import React, { useState, useMemo } from 'react';
import { Search, Filter } from 'lucide-react';

interface Product {
  id: string;
  name: string;
  price: number;
  category: string;
}

const ProductList: React.FC = () => {
  const [searchTerm, setSearchTerm] = useState('');
  const [selectedCategory, setSelectedCategory] = useState('all');

  // Mock data - in real app, fetch from API or store
  const products: Product[] = [
    { id: '1', name: 'Laptop', price: 999, category: 'electronics' },
    { id: '2', name: 'Shirt', price: 29, category: 'clothing' },
    // ... more products
  ];

  const filteredProducts = useMemo(() => {
    return products.filter(product => {
      const matchesSearch = product.name.toLowerCase().includes(searchTerm.toLowerCase());
      const matchesCategory = selectedCategory === 'all' || product.category === selectedCategory;
      return matchesSearch && matchesCategory;
    });
  }, [searchTerm, selectedCategory, products]);

  return (
    <div className="container mx-auto p-6">
      <h1 className="text-3xl font-bold mb-6">Products</h1>

      {/* Filters */}
      <div className="flex gap-4 mb-6">
        <div className="flex-1 relative">
          <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 text-gray-400 w-5 h-5" />
          <input
            type="text"
            placeholder="Search products..."
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="w-full pl-10 pr-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
        </div>

        <select
          value={selectedCategory}
          onChange={(e) => setSelectedCategory(e.target.value)}
          className="px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
        >
          <option value="all">All Categories</option>
          <option value="electronics">Electronics</option>
          <option value="clothing">Clothing</option>
        </select>
      </div>

      {/* Product Grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
        {filteredProducts.map(product => (
          <div key={product.id} className="bg-white rounded-lg shadow p-6 hover:shadow-lg transition-shadow">
            <h3 className="text-lg font-semibold mb-2">{product.name}</h3>
            <p className="text-gray-600 mb-2">{product.category}</p>
            <p className="text-2xl font-bold text-blue-600">${product.price}</p>
          </div>
        ))}
      </div>

      {filteredProducts.length === 0 && (
        <div className="text-center py-12 text-gray-500">
          No products found matching your criteria.
        </div>
      )}
    </div>
  );
};

export default ProductList;
```

## Error Handling

### Error Boundaries

```typescript
import React, { Component, ErrorInfo, ReactNode } from 'react';

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
}

class ErrorBoundary extends Component<Props, State> {
  public state: State = {
    hasError: false
  };

  public static getDerivedStateFromError(_: Error): State {
    return { hasError: true };
  }

  public componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error('Uncaught error:', error, errorInfo);
  }

  public render() {
    if (this.state.hasError) {
      return (
        <div className="min-h-screen flex items-center justify-center">
          <div className="text-center">
            <h1 className="text-2xl font-bold text-red-600 mb-4">
              Something went wrong
            </h1>
            <button
              onClick={() => this.setState({ hasError: false })}
              className="px-4 py-2 bg-blue-500 text-white rounded hover:bg-blue-600"
            >
              Try again
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

export default ErrorBoundary;
```

## TypeScript Types

### Define interfaces for your data

```typescript
interface User {
  id: string;
  name: string;
  email: string;
  role: 'admin' | 'user';
}

interface PageProps {
  initialData?: any;
}

const UserList: React.FC<PageProps> = ({ initialData }) => {
  const [users, setUsers] = useState<User[]>(initialData || []);

  // Type-safe operations
  const addUser = (user: User) => {
    setUsers(prev => [...prev, user]);
  };

  return (
    <div>
      {users.map((user: User) => (
        <div key={user.id}>{user.name}</div>
      ))}
    </div>
  );
};
```

## Important Rules

### ✅ DO:

1. **Always use default export** for page components
2. **Place files in `/src/views/` directory**
3. **Use `.tsx` extension**
4. **Import React at the top**
5. **Use TypeScript types for props and state**
6. **Handle loading and error states**
7. **Use Tailwind CSS for styling**
8. **Implement responsive design**

### ❌ DON'T:

1. **Don't use named exports** - pages must use default export
2. **Don't import packages that aren't available** - check buildAvailablePackages
3. **Don't use absolute imports for project files** - use relative paths like `../components/`
4. **Don't forget error handling** for async operations
5. **Don't mutate state directly** - always create new objects/arrays

## Validation and Hot Reload

Pages are:

1. **Validated** using the same validation system as other files
2. **Type-checked** during compilation
3. **Hot-reloaded** automatically when saved
4. **Rendered dynamically** without page refresh

The ViewRenderer component handles:

- Dynamic compilation of TypeScript to JavaScript
- Error boundaries and error display
- File watching for hot reload
- Automatic route mapping

## Example: Complete Authentication Page

```typescript
import React, { useState } from 'react';
import { useNavigate, Link } from 'react-router-dom';
import { User, Lock, Mail, AlertCircle } from 'lucide-react';
import useAuthStore from '../stores/auth-store';

const LoginPage: React.FC = () => {
  const navigate = useNavigate();
  const { login, isLoading, error } = useAuthStore();

  const [formData, setFormData] = useState({
    email: '',
    password: ''
  });
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({});

  const validateForm = (): boolean => {
    const errors: Record<string, string> = {};

    if (!formData.email) {
      errors.email = 'Email is required';
    } else if (!/\S+@\S+\.\S+/.test(formData.email)) {
      errors.email = 'Email is invalid';
    }

    if (!formData.password) {
      errors.password = 'Password is required';
    } else if (formData.password.length < 6) {
      errors.password = 'Password must be at least 6 characters';
    }

    setValidationErrors(errors);
    return Object.keys(errors).length === 0;
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    if (!validateForm()) {
      return;
    }

    try {
      await login(formData.email, formData.password);
      navigate('/dashboard');
    } catch (err) {
      // Error is handled by the store
    }
  };

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const { name, value } = e.target;
    setFormData(prev => ({ ...prev, [name]: value }));
    // Clear validation error when user starts typing
    if (validationErrors[name]) {
      setValidationErrors(prev => ({ ...prev, [name]: '' }));
    }
  };

  return (
    <div className="min-h-screen bg-gray-50 flex flex-col justify-center py-12 sm:px-6 lg:px-8">
      <div className="sm:mx-auto sm:w-full sm:max-w-md">
        <div className="flex justify-center">
          <User className="w-12 h-12 text-blue-500" />
        </div>
        <h2 className="mt-6 text-center text-3xl font-extrabold text-gray-900">
          Sign in to your account
        </h2>
        <p className="mt-2 text-center text-sm text-gray-600">
          Or{' '}
          <Link to="/register" className="font-medium text-blue-600 hover:text-blue-500">
            create a new account
          </Link>
        </p>
      </div>

      <div className="mt-8 sm:mx-auto sm:w-full sm:max-w-md">
        <div className="bg-white py-8 px-4 shadow sm:rounded-lg sm:px-10">
          {error && (
            <div className="mb-4 p-3 bg-red-50 border border-red-200 rounded-md flex items-start gap-2">
              <AlertCircle className="w-5 h-5 text-red-600 mt-0.5" />
              <span className="text-sm text-red-600">{error}</span>
            </div>
          )}

          <form className="space-y-6" onSubmit={handleSubmit}>
            <div>
              <label htmlFor="email" className="block text-sm font-medium text-gray-700">
                Email address
              </label>
              <div className="mt-1 relative">
                <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none">
                  <Mail className="h-4 w-4 text-gray-400" />
                </div>
                <input
                  id="email"
                  name="email"
                  type="email"
                  autoComplete="email"
                  value={formData.email}
                  onChange={handleChange}
                  className={`pl-10 block w-full shadow-sm sm:text-sm rounded-md ${
                    validationErrors.email
                      ? 'border-red-300 focus:ring-red-500 focus:border-red-500'
                      : 'border-gray-300 focus:ring-blue-500 focus:border-blue-500'
                  }`}
                />
              </div>
              {validationErrors.email && (
                <p className="mt-1 text-sm text-red-600">{validationErrors.email}</p>
              )}
            </div>

            <div>
              <label htmlFor="password" className="block text-sm font-medium text-gray-700">
                Password
              </label>
              <div className="mt-1 relative">
                <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none">
                  <Lock className="h-4 w-4 text-gray-400" />
                </div>
                <input
                  id="password"
                  name="password"
                  type="password"
                  autoComplete="current-password"
                  value={formData.password}
                  onChange={handleChange}
                  className={`pl-10 block w-full shadow-sm sm:text-sm rounded-md ${
                    validationErrors.password
                      ? 'border-red-300 focus:ring-red-500 focus:border-red-500'
                      : 'border-gray-300 focus:ring-blue-500 focus:border-blue-500'
                  }`}
                />
              </div>
              {validationErrors.password && (
                <p className="mt-1 text-sm text-red-600">{validationErrors.password}</p>
              )}
            </div>

            <div className="flex items-center justify-between">
              <div className="flex items-center">
                <input
                  id="remember-me"
                  name="remember-me"
                  type="checkbox"
                  className="h-4 w-4 text-blue-600 focus:ring-blue-500 border-gray-300 rounded"
                />
                <label htmlFor="remember-me" className="ml-2 block text-sm text-gray-900">
                  Remember me
                </label>
              </div>

              <div className="text-sm">
                <Link to="/forgot-password" className="font-medium text-blue-600 hover:text-blue-500">
                  Forgot your password?
                </Link>
              </div>
            </div>

            <div>
              <button
                type="submit"
                disabled={isLoading}
                className="w-full flex justify-center py-2 px-4 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {isLoading ? 'Signing in...' : 'Sign in'}
              </button>
            </div>
          </form>
        </div>
      </div>
    </div>
  );
};

export default LoginPage;
```

## Summary

Creating pages in Tonk involves:

1. Creating a `.tsx` file in `/src/views/`
2. Writing a React component with default export
3. Using available packages and stores
4. Following TypeScript and validation rules
5. Implementing proper error handling and loading states
6. Using Tailwind CSS for styling
7. Leveraging React Router for navigation

The system provides automatic routing, hot reload, and dynamic compilation, making development fast
and efficient.

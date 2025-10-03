# Tonk Component Creation Guidelines

## CRITICAL: No Imports - Everything is Global

When creating React components in Tonk, **NEVER import anything**. All libraries are globally available.

```tsx
// ❌ WRONG - Do not import
import React from 'react';
import { Box, Button } from '@chakra-ui/react';

// ✅ CORRECT - No imports, use directly
const MyComponent = () => {
  return <Box><Button>Click</Button></Box>;
};
```

## Available Libraries

**React & Hooks:**
- `React`, `useState`, `useEffect`, `useCallback`, `useMemo`, `useRef`, `useReducer`, `useContext`, `Fragment`

**Chakra UI v3 (Primary UI Library):**
- ALL components available: `Box`, `Button`, `Heading`, `Text`, `Input`, `Stack`, `VStack`, `HStack`, `Modal`, `Drawer`, `Accordion`, `Tabs`, etc.
- ALL hooks available: `useDisclosure`, `useBreakpoint`, `useBreakpointValue`, etc.

**React Router:**
- `Link`, `NavLink`, `useNavigate`, `useLocation`, `useParams`, `useSearchParams`

**Zustand (for stores):**
- `create`, `sync`

## Basic Component Pattern

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
    <Box p={6} bg="white" borderRadius="lg" shadow="md">
      <Heading size="lg" mb={4}>{title}</Heading>
      <Text mb={4}>Count: {count}</Text>
      <Button 
        colorScheme={isActive ? "green" : "blue"} 
        onClick={handleClick}
      >
        {isActive ? "Active" : "Inactive"}
      </Button>
    </Box>
  );
};

export default MyComponent;
```

## Chakra UI v3 - Compound Component Patterns

Chakra UI v3 uses **compound components** with dot notation. Always use the proper structure:

### Modal Example:
```tsx
const ModalExample: React.FC = () => {
  const { isOpen, onOpen, onClose } = useDisclosure();

  return (
    <Box>
      <Button onClick={onOpen}>Open Modal</Button>

      <Modal.Root open={isOpen} onOpenChange={({ open }) => !open && onClose()}>
        <Modal.Backdrop />
        <Modal.Positioner>
          <Modal.Content>
            <Modal.Header>
              <Modal.Title>Modal Title</Modal.Title>
              <Modal.CloseTrigger />
            </Modal.Header>
            <Modal.Body>
              <Text>Modal content here</Text>
            </Modal.Body>
            <Modal.Footer>
              <Button onClick={onClose}>Close</Button>
            </Modal.Footer>
          </Modal.Content>
        </Modal.Positioner>
      </Modal.Root>
    </Box>
  );
};
```

### Common Chakra Components:

**Layout:** `Box`, `Container`, `Flex`, `Grid`, `Stack`, `VStack`, `HStack`, `Center`, `Spacer`

**Typography:** `Heading`, `Text`

**Forms:** `Input`, `Textarea`, `Select`, `Checkbox`, `Radio`, `Switch`, `Field`

**Buttons:** `Button`, `IconButton`

**Feedback:** `Alert`, `Spinner`, `ProgressCircle`, `ProgressBar`

**Overlay:** `Modal`, `Drawer`, `Popover`, `Tooltip`

**Disclosure:** `Accordion`, `Tabs`, `Collapsible`

**Data Display:** `Badge`, `Card`, `Table`, `Tag`, `Avatar`, `Image`

## Store Integration - CRITICAL for Forms

### Rule: Use Selective Subscriptions in Parent Components

When a parent component displays a child form, **DO NOT** subscribe to the entire store state or arrays that change frequently. This causes the child to re-render and **inputs lose focus**.

```tsx
// ❌ BAD - Causes input focus loss
const ContactPage: React.FC = () => {
  const { submissions } = useContactStore(); // Re-renders on every change!
  
  return (
    <Box>
      <ContactForm /> {/* Will lose focus when submissions update */}
      <Text>Total: {submissions.length}</Text>
    </Box>
  );
};

// ✅ GOOD - Selective subscription
const ContactPage: React.FC = () => {
  const count = useContactStore((state) => state.submissions.length);
  
  return (
    <Box>
      <ContactForm /> {/* Won't re-render unnecessarily */}
      <Text>Total: {count}</Text>
    </Box>
  );
};
```

### Using Stores in Components:

Stores are globally available after compilation. Access them directly:

```tsx
const MyComponent: React.FC = () => {
  const { data, isLoading, fetchData } = useExampleStore();

  useEffect(() => {
    fetchData();
  }, []);

  if (isLoading) return <Spinner />;

  return (
    <Box p={4}>
      <Text>{data}</Text>
    </Box>
  );
};
```

## Navigation - CRITICAL Rules

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
<Link to="/about" color="blue.500">About Us</Link>
<Button as={Link} to="/products">Products</Button>

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

  return <Button onClick={handleClick}>Go to Dashboard</Button>;
};
```

## User Feedback - NO Toast Notifications

**Toast notifications are NOT supported.** Use Alert components instead:

```tsx
const MyComponent: React.FC = () => {
  const [showSuccess, setShowSuccess] = useState(false);

  const handleAction = () => {
    // Do something
    setShowSuccess(true);
    setTimeout(() => setShowSuccess(false), 3000);
  };

  return (
    <Box>
      {showSuccess && (
        <Alert.Root status="success" mb={4}>
          <Alert.Indicator />
          <Alert.Title>Success!</Alert.Title>
          <Alert.Description>Action completed successfully</Alert.Description>
        </Alert.Root>
      )}
      <Button onClick={handleAction}>Do Action</Button>
    </Box>
  );
};
```

## Critical Checklist

Before submitting any component, verify:

- [ ] **NO imports** - All libraries used directly
- [ ] **Default export** - Component must export default
- [ ] **TypeScript props** - Interface defined for props
- [ ] **Chakra UI compound patterns** - Using Modal.Root, Drawer.Content, etc.
- [ ] **Selective store subscriptions** - Parent components use selectors
- [ ] **Absolute navigation paths** - All routes start with `/`
- [ ] **Alert for feedback** - No toast/toaster usage
- [ ] **Managed form inputs** - If using forms with stores

## Component Requirements

1. **Must use TypeScript** - Define interfaces for props
2. **Must use React.FC<Props>** - Type the component properly
3. **Must export default** - Required for hot reloading
4. **PascalCase names** - Component and file names
5. **File location** - `/src/components/ComponentName.tsx`
6. **Hooks at top level** - Never in conditions or loops
7. **Event handlers** - Use arrow functions or useCallback

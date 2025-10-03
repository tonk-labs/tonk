# Tonk Component Creation Guidelines for LLM Agents

## Critical Component Format Requirements

When creating React components in the Tonk environment, you MUST follow these exact patterns.
Components are compiled and hot-reloaded through our bespoke HMR system.

## Component File Structure

### Basic Function Component Pattern

```tsx
interface ComponentProps {
  prop1: string;
  prop2?: number;
  onAction?: (value: string) => void;
}

const ComponentName: React.FC<ComponentProps> = ({ prop1, prop2 = 0, onAction }) => {
  const [state, setState] = useState<string>('');

  useEffect(() => {
    // Effect logic
  }, [dependency]);

  const handleClick = () => {
    if (onAction) {
      onAction(state);
    }
  };

  return (
    <Box p={4} bg="white" borderRadius="lg" shadow="md">
      <Heading>{prop1}</Heading>
      <Button onClick={handleClick}>Click me</Button>
    </Box>
  );
};

export default ComponentName;
```

## Critical Rules for Components

### 1. Available Libraries (NO IMPORTS NEEDED)

The following are available globally without imports:

**React & Hooks:**
- `React`, `useState`, `useEffect`, `useCallback`, `useMemo`, `useRef`, `useReducer`, `useContext`, `Fragment`

**Chakra UI Components & Hooks (USE THIS AS PRIMARY UI LIBRARY):**
- All Chakra UI components: `Box`, `Button`, `Heading`, `Text`, `Input`, `Stack`, `Flex`, `Grid`, etc.
- All Chakra UI hooks: `useDisclosure`, `useToast`, `useColorMode`, etc.
- Chakra UI is the primary component library - use it for all UI elements

**React Router:**
- `Link`, `NavLink`, `useNavigate`, `useLocation`, `useParams`, `useSearchParams`

**Zustand (for stores):**
- `create`, `sync`

### 2. Component Definition

- **MUST** use typed props with TypeScript interfaces
- **MUST** use `React.FC<Props>` or function declaration with typed props
- **MUST** use default export
- Component name should be PascalCase
- **NO IMPORTS** - all libraries are globally available

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
  const { data, isLoading, fetchData } = useExampleStore();

  useEffect(() => {
    fetchData();
  }, []);

  if (isLoading) {
    return <Spinner />;
  }

  return (
    <Box p={4} bg="white" borderRadius="lg" shadow="md">
      <Heading size="lg" mb={4}>{title}</Heading>
      <Text>{data}</Text>
    </Box>
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
    navigate('/about');
    navigate('/products', { replace: true });
  };

  return (
    <Stack spacing={4}>
      <Link to="/about" color="blue.500">
        About Us
      </Link>

      <Button as={Link} to="/products" colorScheme="blue">
        View Products
      </Button>

      <NavLink
        to="/dashboard"
        className={({ isActive }) => (isActive ? 'text-blue-600 font-bold' : 'text-gray-600')}
      >
        Dashboard
      </NavLink>

      <Button onClick={handleNavigation}>Go to About</Button>
    </Stack>
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

## UI Component Library - Chakra UI

**USE CHAKRA UI AS THE PRIMARY COMPONENT LIBRARY**

All Chakra UI components and hooks are available globally without imports:

### Common Components:
- Layout: `Box`, `Container`, `Flex`, `Grid`, `Stack`, `VStack`, `HStack`, `Wrap`, `Center`, `Spacer`
- Typography: `Heading`, `Text`
- Forms: `Input`, `Textarea`, `Select`, `Checkbox`, `Radio`, `Switch`, `FormControl`, `FormLabel`
- Buttons: `Button`, `IconButton`, `ButtonGroup`
- Feedback: `Alert`, `Spinner`, `Toast`, `Modal`, `Drawer`, `Popover`
- Data Display: `Badge`, `Card`, `Table`, `Tag`, `Avatar`, `Image`
- Navigation: `Breadcrumb`, `Tabs`, `Menu`

### Common Hooks:
- `useDisclosure`, `useToast`, `useColorMode`, `useBreakpointValue`

### Example with Chakra UI:

```tsx
const UserCard: React.FC<{ name: string; email: string }> = ({ name, email }) => {
  const { isOpen, onOpen, onClose } = useDisclosure();
  const toast = useToast();

  const handleSave = () => {
    toast({
      title: "Profile saved",
      status: "success",
      duration: 3000,
    });
  };

  return (
    <Box>
      <Card>
        <CardBody>
          <VStack align="start" spacing={3}>
            <Heading size="md">{name}</Heading>
            <Text color="gray.600">{email}</Text>
            <Button colorScheme="blue" onClick={onOpen}>
              Edit Profile
            </Button>
          </VStack>
        </CardBody>
      </Card>

      <Modal isOpen={isOpen} onClose={onClose}>
        <ModalOverlay />
        <ModalContent>
          <ModalHeader>Edit Profile</ModalHeader>
          <ModalCloseButton />
          <ModalBody>
            <VStack spacing={4}>
              <FormControl>
                <FormLabel>Name</FormLabel>
                <Input defaultValue={name} />
              </FormControl>
              <FormControl>
                <FormLabel>Email</FormLabel>
                <Input defaultValue={email} />
              </FormControl>
            </VStack>
          </ModalBody>
          <ModalFooter>
            <Button variant="ghost" mr={3} onClick={onClose}>
              Cancel
            </Button>
            <Button colorScheme="blue" onClick={handleSave}>
              Save
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>
    </Box>
  );
};
```

# IMPORTANT

## Forms and Inputs

When you create input of forms that use a store- the input boxes MUST be managed or otherwise the
input boxes will constantly reset after typing and the users can't use your forms.

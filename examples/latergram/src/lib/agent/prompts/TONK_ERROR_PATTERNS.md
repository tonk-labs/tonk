# Common Error Patterns & How to Fix Them

This document is shown when a component has errors. Review these patterns to avoid repeating mistakes.

## React Error #130: "Objects are not valid as a React child"

**Cause:** Passing complex objects or wrapping simple elements where React expects simple values.

### Button Icon Issue:

```tsx
// ❌ WRONG - Wrapping emoji in Text component
<Button leftIcon={<Text>▶️</Text>}>Start</Button>

// ✅ CORRECT - Just use emoji directly in button text
<Button>▶️ Start</Button>

// ✅ ALSO CORRECT - Use simple span if you need leftIcon
<Button leftIcon={<span>▶️</span>}>Start</Button>
```

**Rule:** `leftIcon` and `rightIcon` expect simple React elements, not wrapped components.

## Input Focus Loss

**Symptom:** When typing in form fields, the cursor jumps or loses focus after every keystroke.

**Cause:** Parent component is subscribing to store state that changes frequently, causing re-renders that unmount and remount the child component with inputs.

```tsx
// ❌ WRONG - Parent subscribes to entire array
const ContactPage: React.FC = () => {
  const { submissions } = useContactStore(); // Re-renders every time array changes!
  
  return (
    <Box>
      <ContactForm /> {/* Will lose focus when parent re-renders */}
      <Text>Total: {submissions.length}</Text>
    </Box>
  );
};

// ✅ CORRECT - Use selective subscription
const ContactPage: React.FC = () => {
  const count = useContactStore((state) => state.submissions.length);
  
  return (
    <Box>
      <ContactForm /> {/* Won't re-render unless its own state changes */}
      <Text>Total: {count}</Text>
    </Box>
  );
};
```

**Rules for Store Subscriptions:**

1. **Never subscribe to arrays/objects if you only need derived values:**
   ```tsx
   // ❌ BAD
   const { items } = useStore();
   const count = items.length;
   
   // ✅ GOOD
   const count = useStore((state) => state.items.length);
   ```

2. **Use selectors to extract only what you need:**
   ```tsx
   // ❌ BAD - subscribes to entire user object
   const { user } = useStore();
   const userName = user.name;
   
   // ✅ GOOD - only subscribes to name
   const userName = useStore((state) => state.user.name);
   ```

3. **Parent components with forms MUST use minimal subscriptions:**
   - Subscribe only to counts, booleans, or primitive values
   - Move array/object subscriptions to child components
   - Use `React.memo()` on form components if parent must re-render

## Toast/Toaster Errors: "c is not a function" or similar

**Cause:** Trying to use toast notifications, which are not supported in dynamic components due to Chakra UI v3 context requirements.

```tsx
// ❌ WRONG - Toast API not supported
toaster.success({ title: "Success!" });
toaster.error({ title: "Error!" });

// ✅ CORRECT - Use Alert component with state
const [showSuccess, setShowSuccess] = useState(false);

const handleAction = () => {
  // Do action
  setShowSuccess(true);
  setTimeout(() => setShowSuccess(false), 3000);
};

return (
  <Box>
    {showSuccess && (
      <Alert.Root status="success" mb={4}>
        <Alert.Indicator />
        <Alert.Title>Success!</Alert.Title>
      </Alert.Root>
    )}
    <Button onClick={handleAction}>Do Action</Button>
  </Box>
);
```

## ProgressCircle/ProgressBar Errors

**Cause:** Using wrong API for Chakra UI v3.

```tsx
// ❌ WRONG - Old v2 API
<ProgressCircle value={75} size="lg">
  <Text>75%</Text>
</ProgressCircle>

// ✅ CORRECT - v3 compound component pattern
<ProgressCircle.Root value={75} size="lg">
  <ProgressCircle.Circle>
    <ProgressCircle.Track />
    <ProgressCircle.Range />
  </ProgressCircle.Circle>
  <ProgressCircle.ValueText>75%</ProgressCircle.ValueText>
</ProgressCircle.Root>
```

**Rule:** Always use compound pattern: `ComponentName.Root` → `ComponentName.SubComponent`

## Navigation Not Working

**Symptom:** Links don't work or cause page reload.

**Cause:** Using relative paths or wrong component.

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

## Store/Component Not Found Errors

**Symptom:** "useExampleStore is not defined" or similar.

**Cause:** Trying to import the store or component.

```tsx
// ❌ WRONG - Trying to import
import { useExampleStore } from '../stores/example';

// ✅ CORRECT - No import, it's globally available
const MyComponent = () => {
  const { data } = useExampleStore(); // Directly available
  return <Text>{data}</Text>;
};
```

**Rule:** Never import anything. All stores and components are globally available after compilation.

## Chakra Component Not Rendering

**Symptom:** Component renders but Chakra UI styling doesn't work or component is invisible.

**Possible causes:**

1. **Wrong compound component structure:**
   ```tsx
   // ❌ WRONG
   <Modal isOpen={true}>
     <ModalContent>...</ModalContent>
   </Modal>
   
   // ✅ CORRECT
   <Modal.Root open={true}>
     <Modal.Backdrop />
     <Modal.Positioner>
       <Modal.Content>...</Modal.Content>
     </Positioner>
   </Modal.Root>
   ```

2. **Missing required props:**
   ```tsx
   // ❌ WRONG - Missing value prop
   <ProgressCircle.Root size="lg">
     <ProgressCircle.Circle />
   </ProgressCircle.Root>
   
   // ✅ CORRECT - Include required props
   <ProgressCircle.Root value={75} size="lg">
     <ProgressCircle.Circle>
       <ProgressCircle.Track />
       <ProgressCircle.Range />
     </ProgressCircle.Circle>
   </ProgressCircle.Root>
   ```

## Forms Not Updating Store

**Symptom:** Typing in inputs but store doesn't update, or updates don't persist.

**Cause:** Not properly connecting input value and onChange to store.

```tsx
// ❌ WRONG - Not controlled
<Input placeholder="Name" />

// ✅ CORRECT - Controlled input with store
const FormComponent = () => {
  const name = useFormStore((state) => state.name);
  const setName = useFormStore((state) => state.setName);
  
  return (
    <Input 
      value={name}
      onChange={(e) => setName(e.target.value)}
      placeholder="Name"
    />
  );
};
```

**Rule:** All form inputs must be controlled (value + onChange) and connected to store state.

## Import/Require Errors

**Symptom:** "Cannot find module" or "import is not defined"

**Cause:** Trying to use import/require statements.

```tsx
// ❌ WRONG - No imports allowed
import React from 'react';
import { Box } from '@chakra-ui/react';
const something = require('./utils');

// ✅ CORRECT - No imports needed
const MyComponent = () => {
  return <Box>Everything is global</Box>;
};

export default MyComponent;
```

**Rule:** NEVER use import or require. Everything is globally available.

## TypeScript Errors

**Symptom:** Type errors during compilation.

**Common issues:**

1. **Props not typed:**
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

2. **Event handlers not typed:**
   ```tsx
   // ❌ WRONG
   const handleChange = (e) => { ... }
   
   // ✅ CORRECT
   const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => { ... }
   ```

## Quick Checklist When You Get an Error

1. **Did you use any imports?** → Remove them all
2. **Are you using Chakra v3 compound patterns?** → Check Modal.Root, Drawer.Content, etc.
3. **Is a parent component causing re-renders?** → Use selective store subscriptions
4. **Did you try to use toast notifications?** → Use Alert instead
5. **Are paths absolute?** → Use `/about` not `about`
6. **Are inputs controlled?** → Connect value and onChange to store
7. **Did you export default?** → Component must export default

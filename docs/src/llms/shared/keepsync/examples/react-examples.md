# React Keepsync Examples

This section shows complete examples of how to use keepsync in React applications. These examples demonstrate a collaborative todo application with real-time synchronization.

## Todo Store

A complete Zustand store with keepsync synchronization:

```typescript
{{#include ../../../../../../packages/create/templates/react/instructions/keepsync/examples/react/stores/todoStore.ts}}
```

## Todo List Component

A React component that displays and manages todos:

```typescript
{{#include ../../../../../../packages/create/templates/react/instructions/keepsync/examples/react/components/TodoList.tsx}}
```

## Add Todo Component

A React component for adding new todos:

```typescript
{{#include ../../../../../../packages/create/templates/react/instructions/keepsync/examples/react/components/AddTodo.tsx}}
```

## Main App Component

The main application component that brings everything together:

```typescript
{{#include ../../../../../../packages/create/templates/react/instructions/keepsync/examples/react/App.tsx}}
```

## Key Concepts Demonstrated

1. **Sync Middleware**: Using `sync()` to create a collaborative Zustand store
2. **Document ID**: Using `docId` to identify the shared document
3. **Real-time Updates**: Changes are automatically synchronized across all clients
4. **Component Integration**: React components seamlessly use the synced store
5. **Connection Management**: Handling connection status and reconnection scenarios

## Running the Example

To run this example:

1. Create a new Tonk React app: `tonk create` (choose React)
2. Replace the generated files with the code above
3. Start the development server: `pnpm dev`
4. Open multiple browser windows to see real-time collaboration

The todos will be synchronized in real-time across all connected clients! 
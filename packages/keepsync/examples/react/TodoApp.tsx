import React, {useEffect, useState} from 'react';
import {configureSyncEngine} from '../../src/core/syncConfig';
import {UseBoundStore, StoreApi, create} from 'zustand';
import {sync} from '../../src/middleware';

// Define the Todo type
interface Todo {
  id: string;
  text: string;
  completed: boolean;
}

// Define the store type
interface TodoStore {
  todos: Todo[];
  addTodo: (text: string) => void;
  toggleTodo: (id: string) => void;
  removeTodo: (id: string) => void;
  clearCompleted: () => void;
}

// Initialize the sync engine
export function initializeTodoSync() {
  configureSyncEngine({
    url: 'ws://localhost:4080/sync',
    name: 'TodoApplication',
    dbName: 'todo_app_db',
    onSync: docId => console.log(`Todo document ${docId} synced`),
    onError: error => console.error('Todo sync error:', error),
  });
}

// Create the todo store
export const useTodoStore = create<TodoStore>(
  sync(
    set => ({
      todos: [],

      addTodo: (text: string) => {
        set(state => ({
          todos: [
            ...state.todos,
            {
              id: Date.now().toString(),
              text,
              completed: false,
            },
          ],
        }));
      },

      toggleTodo: (id: string) => {
        set(state => ({
          todos: state.todos.map(todo =>
            todo.id === id ? {...todo, completed: !todo.completed} : todo,
          ),
        }));
      },

      removeTodo: (id: string) => {
        set(state => ({
          todos: state.todos.filter(todo => todo.id !== id),
        }));
      },

      clearCompleted: () => {
        set(state => ({
          todos: state.todos.filter(todo => !todo.completed),
        }));
      },
    }),
    {
      docId: 'todos-doc',
    },
  ),
);

// TodoItem component
const TodoItem: React.FC<{
  todo: Todo;
  onToggle: () => void;
  onRemove: () => void;
}> = ({todo, onToggle, onRemove}) => {
  return (
    <li
      style={{
        display: 'flex',
        alignItems: 'center',
        marginBottom: '8px',
        textDecoration: todo.completed ? 'line-through' : 'none',
        opacity: todo.completed ? 0.6 : 1,
      }}
    >
      <input
        type="checkbox"
        checked={todo.completed}
        onChange={onToggle}
        style={{marginRight: '8px'}}
      />
      <span style={{flex: 1}}>{todo.text}</span>
      <button
        onClick={onRemove}
        style={{
          background: 'transparent',
          border: 'none',
          color: '#ff6b6b',
          cursor: 'pointer',
          fontSize: '16px',
        }}
      >
        ×
      </button>
    </li>
  );
};

// TodoApp component
export const TodoApp: React.FC = () => {
  const [store, setStore] = useState<UseBoundStore<StoreApi<TodoStore>> | null>(
    null,
  );
  const [newTodo, setNewTodo] = useState('');

  // Initialize the store
  useEffect(() => {
    // Initialize sync engine
    initializeTodoSync();

    // Set the store
    setStore(useTodoStore);

    // Cleanup on unmount
    return () => {
      // Optional: Close sync engine when component unmounts
      // closeSyncEngine();
    };
  }, []);

  // If store isn't ready yet, show loading
  if (!store) {
    return <div>Loading todo sync...</div>;
  }

  // Use the store
  const TodoList = () => {
    const {todos, addTodo, toggleTodo, removeTodo, clearCompleted} = store();

    const handleSubmit = (e: React.FormEvent) => {
      e.preventDefault();
      if (newTodo.trim()) {
        addTodo(newTodo.trim());
        setNewTodo('');
      }
    };

    return (
      <div
        style={{
          maxWidth: '500px',
          margin: '0 auto',
          padding: '20px',
          fontFamily: 'Arial, sans-serif',
        }}
      >
        <h1 style={{textAlign: 'center', color: '#333'}}>Synced Todo List</h1>

        <form
          onSubmit={handleSubmit}
          style={{
            display: 'flex',
            marginBottom: '20px',
          }}
        >
          <input
            type="text"
            value={newTodo}
            onChange={e => setNewTodo(e.target.value)}
            placeholder="What needs to be done?"
            style={{
              flex: 1,
              padding: '8px 12px',
              fontSize: '16px',
              border: '1px solid #ddd',
              borderRadius: '4px 0 0 4px',
            }}
          />
          <button
            type="submit"
            style={{
              padding: '8px 16px',
              background: '#4caf50',
              color: 'white',
              border: 'none',
              borderRadius: '0 4px 4px 0',
              cursor: 'pointer',
            }}
          >
            Add
          </button>
        </form>

        {todos.length > 0 ? (
          <>
            <ul
              style={{
                listStyle: 'none',
                padding: 0,
                margin: 0,
              }}
            >
              {todos.map((todo: Todo) => (
                <TodoItem
                  key={todo.id}
                  todo={todo}
                  onToggle={() => toggleTodo(todo.id)}
                  onRemove={() => removeTodo(todo.id)}
                />
              ))}
            </ul>

            <div
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                marginTop: '20px',
                color: '#666',
                fontSize: '14px',
              }}
            >
              <span>
                {todos.filter((t: Todo) => !t.completed).length} items left
              </span>
              <button
                onClick={clearCompleted}
                style={{
                  background: 'transparent',
                  border: 'none',
                  color: '#999',
                  cursor: 'pointer',
                  textDecoration: 'underline',
                }}
              >
                Clear completed
              </button>
            </div>
          </>
        ) : (
          <p style={{textAlign: 'center', color: '#999'}}>
            No todos yet. Add one above!
          </p>
        )}

        <div
          style={{
            marginTop: '30px',
            padding: '15px',
            background: '#f5f5f5',
            borderRadius: '4px',
            fontSize: '14px',
            color: '#666',
          }}
        >
          <p style={{margin: '0 0 10px 0'}}>
            <strong>Sync Status:</strong> Active
          </p>
          <p style={{margin: 0}}>
            Changes are automatically synchronized across all connected clients.
            Try opening this app in multiple browser windows to see real-time
            updates!
          </p>
        </div>
      </div>
    );
  };

  return <TodoList />;
};

// Usage in your application:
// import { TodoApp } from './examples/react/TodoApp';
//
// function App() {
//   return (
//     <div className="App">
//       <TodoApp />
//     </div>
//   );
// }

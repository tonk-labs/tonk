import React, { useState } from 'react';
import AddTodo from './components/AddTodo';
import TodoList from './components/TodoList';

function App() {
  const [connectionStatus, setConnectionStatus] = useState<
    'connected' | 'disconnected'
  >('connected');

  // Handle disconnecting from the sync engine
  const handleDisconnect = () => {
    setConnectionStatus('disconnected');
  };

  // Handle reconnecting to the sync engine
  const handleReconnect = () => {
    // This will reinitialize the sync engine on the next store access
    setConnectionStatus('connected');
  };

  return (
    <div className="container">
      <h1>Collaborative Todo List</h1>
      <p>
        Status:{' '}
        <span
          style={{ color: connectionStatus === 'connected' ? 'green' : 'red' }}
        >
          {connectionStatus}
        </span>
        {connectionStatus === 'connected' ? (
          <button onClick={handleDisconnect} style={{ marginLeft: '1rem' }}>
            Disconnect
          </button>
        ) : (
          <button onClick={handleReconnect} style={{ marginLeft: '1rem' }}>
            Reconnect
          </button>
        )}
      </p>

      <AddTodo />
      <TodoList />

      <p>
        <small>
          Changes are automatically synced across all connected clients.
          <br />
          Open this app in multiple windows to see real-time collaboration in
          action.
        </small>
      </p>
    </div>
  );
}

export default App;

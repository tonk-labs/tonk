import React, { useEffect, useState } from 'react';
import { useTodoStore } from '../stores/todoStore';
import { AddTodo } from '../components/AddTodo';
import { TodoItem } from '../components/TodoItem';
import { Wifi, WifiOff, Loader } from 'lucide-react';

export const TodoApp: React.FC = () => {
  const {
    todos,
    isInitialized,
    addTodo,
    toggleTodo,
    deleteTodo,
    initialize,
    connectSync,
  } = useTodoStore();

  const [syncStatus, setSyncStatus] = useState<'disconnected' | 'connecting' | 'connected'>('disconnected');
  const [syncUrl, setSyncUrl] = useState('ws://localhost:7777/sync');

  useEffect(() => {
    if (!isInitialized) {
      initialize();
    }
  }, [isInitialized, initialize]);

  const handleConnectSync = async () => {
    if (!isInitialized) return;
    
    setSyncStatus('connecting');
    try {
      await connectSync(syncUrl);
      setSyncStatus('connected');
    } catch (error) {
      console.error('Sync connection failed:', error);
      setSyncStatus('disconnected');
    }
  };

  const completedCount = todos.filter(todo => todo.completed).length;
  const totalCount = todos.length;

  if (!isInitialized) {
    return (
      <div className="min-h-screen bg-gray-50 flex items-center justify-center">
        <div className="flex items-center gap-2 text-gray-600">
          <Loader className="animate-spin" size={20} />
          Initializing Todo App...
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50 py-8">
      <div className="max-w-2xl mx-auto px-4">
        <div className="bg-white rounded-xl shadow-sm p-6 mb-6">
          <div className="flex items-center justify-between mb-6">
            <div>
              <h1 className="text-3xl font-bold text-gray-900">Todo App</h1>
              <p className="text-gray-600 mt-1">
                {totalCount === 0 ? 'No todos yet' : `${completedCount} of ${totalCount} completed`}
              </p>
            </div>
            
            <div className="flex items-center gap-2">
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={syncUrl}
                  onChange={(e) => setSyncUrl(e.target.value)}
                  placeholder="Sync server URL"
                  className="px-3 py-1 text-sm border border-gray-300 rounded focus:outline-none focus:ring-1 focus:ring-blue-500"
                  disabled={syncStatus === 'connecting'}
                />
                <button
                  onClick={handleConnectSync}
                  disabled={syncStatus === 'connecting'}
                  className={`px-3 py-1 text-sm rounded flex items-center gap-1 ${
                    syncStatus === 'connected'
                      ? 'bg-green-100 text-green-700'
                      : syncStatus === 'connecting'
                      ? 'bg-yellow-100 text-yellow-700'
                      : 'bg-gray-100 text-gray-700 hover:bg-gray-200'
                  }`}
                >
                  {syncStatus === 'connected' ? (
                    <>
                      <Wifi size={14} />
                      Connected
                    </>
                  ) : syncStatus === 'connecting' ? (
                    <>
                      <Loader className="animate-spin" size={14} />
                      Connecting...
                    </>
                  ) : (
                    <>
                      <WifiOff size={14} />
                      Connect
                    </>
                  )}
                </button>
              </div>
            </div>
          </div>

          <AddTodo onAdd={addTodo} />

          <div className="space-y-2">
            {todos.length === 0 ? (
              <div className="text-center py-8 text-gray-500">
                <p>No todos yet. Add one above to get started!</p>
              </div>
            ) : (
              todos
                .sort((a, b) => b.createdAt - a.createdAt)
                .map(todo => (
                  <TodoItem
                    key={todo.id}
                    todo={todo}
                    onToggle={toggleTodo}
                    onDelete={deleteTodo}
                  />
                ))
            )}
          </div>

          {syncStatus === 'connected' && (
            <div className="mt-6 p-3 bg-green-50 border border-green-200 rounded-lg">
              <p className="text-sm text-green-800">
                🌐 Real-time sync is active! Open this app in multiple tabs to see changes sync automatically.
              </p>
            </div>
          )}
        </div>

        <div className="bg-white rounded-xl shadow-sm p-4">
          <h3 className="text-lg font-semibold mb-2">About This Todo App</h3>
          <ul className="text-sm text-gray-600 space-y-1">
            <li>• Data persists in IndexedDB using Tonk Core VFS</li>
            <li>• Real-time sync between browser tabs and devices</li>
            <li>• Offline-first with conflict resolution</li>
            <li>• Built with React, Zustand, and Tailwind CSS</li>
          </ul>
        </div>
      </div>
    </div>
  );
};
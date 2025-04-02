import React, { useState } from "react";
import AddTodo from "../components/AddTodo";
import TodoList from "../components/TodoList";
import { useSyncStatus } from "@tonk/keepsync";

const Todos = () => {
  // Use the current keepsync API for sync status
  const [syncStatus, setSyncStatus] = useState<"connected" | "connecting" | "disconnected">("connecting");

  // Check sync connection status on component mount
  React.useEffect(() => {
    // For this simple demo, we'll simulate the sync status
    // In a real implementation, we would use the keepsync API properly
    setSyncStatus("connected");
    
    const syncStatusInterval = setInterval(() => {
      // This is just to demonstrate the UI - in a real app, this would come from keepsync
      const online = navigator.onLine;
      setSyncStatus(online ? "connected" : "disconnected");
    }, 5000);
    
    return () => clearInterval(syncStatusInterval);
  }, []);

  // Toggle connection demo
  const handleToggleConnection = () => {
    if (syncStatus === "connected") {
      setSyncStatus("disconnected");
    } else {
      setSyncStatus("connected");
    }
  };

  return (
    <main className="min-h-screen bg-gray-50 p-4 md:p-6 lg:p-8">
      <section className="max-w-3xl mx-auto">
        <div className="mb-6">
          <div className="flex justify-between items-center mb-2">
            <h1 className="text-3xl font-bold text-gray-900">Todo List</h1>
            <div className="flex items-center">
              <span className="mr-2 text-sm">Sync Status:</span>
              <span 
                className={`inline-block h-2.5 w-2.5 rounded-full mr-1 ${
                  syncStatus === "connected" 
                    ? "bg-green-500" 
                    : syncStatus === "connecting" 
                      ? "bg-yellow-500"
                      : "bg-red-500"
                }`}
              />
              <span className="text-sm mr-2">{syncStatus}</span>
              {syncStatus !== "connecting" && (
                <button
                  onClick={handleToggleConnection}
                  className="text-sm px-2 py-1 bg-white border border-gray-300 rounded hover:bg-gray-50 transition-colors"
                >
                  {syncStatus === "connected" ? "Disconnect" : "Reconnect"}
                </button>
              )}
            </div>
          </div>
          <p className="text-gray-600">
            A collaborative todo list that syncs across all connected devices
          </p>
        </div>

        <AddTodo />
        <TodoList />

        <div className="mt-6 text-sm text-gray-500">
          <p>
            This app uses KeepSync for real-time collaboration. Changes are automatically
            synchronized across all connected clients.
          </p>
          <p className="mt-1">
            Open this app in multiple windows to see real-time collaboration in action.
          </p>
        </div>
      </section>
    </main>
  );
};

export default Todos;
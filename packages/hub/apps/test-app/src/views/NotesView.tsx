import React from "react";
import NotesApp from "../components/NotesApp";

/**
 * A view component that displays the Notes application
 */
const NotesView = () => {
  return (
    <main className="min-h-screen bg-gray-50 p-4 md:p-6 lg:p-8">
      <section className="max-w-6xl mx-auto">
        <div className="mb-6">
          <h1 className="text-3xl font-bold text-gray-900">Notes App</h1>
          <p className="text-gray-600 mt-2">
            Create, edit, and manage your notes. All changes are automatically synced across devices.
          </p>
        </div>
        
        <div className="bg-white rounded-lg shadow-sm h-[600px]">
          <NotesApp title="My Notes" />
        </div>
        
        <div className="mt-6 bg-blue-50 p-4 rounded-lg border border-blue-100 text-sm text-blue-800">
          <p>
            <strong>Pro Tip:</strong> Open this app in multiple windows to see real-time synchronization in action!
          </p>
        </div>
      </section>
    </main>
  );
};

export default NotesView;
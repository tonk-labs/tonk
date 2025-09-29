import React, { useEffect } from 'react';
import { useTodoStore } from '../stores/todoStore';
import { AddTodo } from '../components/AddTodo';
import { TodoItem } from '../components/TodoItem';
import { Loader } from 'lucide-react';

export const TodoApp: React.FC = () => {
  const { todos, isInitialized, addTodo, toggleTodo, deleteTodo, initialize } =
    useTodoStore();

  useEffect(() => {
    if (!isInitialized) {
      initialize();
    }
  }, [isInitialized, initialize]);

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
                {totalCount === 0
                  ? 'No todos yet'
                  : `${completedCount} of ${totalCount} completed`}
              </p>
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
        </div>
      </div>
    </div>
  );
};


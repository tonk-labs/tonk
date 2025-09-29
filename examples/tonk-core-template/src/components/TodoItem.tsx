import React from 'react';
import { Trash2, Check } from 'lucide-react';
import { Todo } from '../stores/todoStore';

interface TodoItemProps {
  todo: Todo;
  onToggle: (id: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
}

export const TodoItem: React.FC<TodoItemProps> = ({ todo, onToggle, onDelete }) => {
  const handleToggle = () => onToggle(todo.id);
  const handleDelete = () => onDelete(todo.id);

  return (
    <div className={`flex items-center gap-3 p-4 bg-white rounded-lg shadow-sm border ${todo.completed ? 'opacity-60' : ''}`}>
      <button
        onClick={handleToggle}
        className={`flex-shrink-0 w-5 h-5 rounded border-2 flex items-center justify-center transition-colors ${
          todo.completed
            ? 'bg-green-500 border-green-500 text-white'
            : 'border-gray-300 hover:border-green-400'
        }`}
      >
        {todo.completed && <Check size={12} />}
      </button>
      
      <span className={`flex-1 ${todo.completed ? 'line-through text-gray-500' : 'text-gray-900'}`}>
        {todo.text}
      </span>
      
      <div className="text-xs text-gray-400">
        {new Date(todo.createdAt).toLocaleDateString()}
      </div>
      
      <button
        onClick={handleDelete}
        className="flex-shrink-0 p-1 text-gray-400 hover:text-red-500 transition-colors"
        title="Delete todo"
      >
        <Trash2 size={16} />
      </button>
    </div>
  );
};
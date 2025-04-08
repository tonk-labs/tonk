import React, { useState } from "react";
import { Todo, FilterOption, SortOption, useTodoStore } from "../stores/todoStore";

const TodoItem: React.FC<{
  todo: Todo;
  onToggle: (id: string) => void;
  onDelete: (id: string) => void;
  onEdit: (id: string, text: string) => void;
  onUpdatePriority: (id: string, priority: Todo["priority"]) => void;
  onUpdateDueDate: (id: string, dueDate?: string) => void;
}> = ({ todo, onToggle, onDelete, onEdit, onUpdatePriority, onUpdateDueDate }) => {
  const [isEditing, setIsEditing] = useState(false);
  const [editedText, setEditedText] = useState(todo.text);
  const [isExpanded, setIsExpanded] = useState(false);

  const handleEditSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (editedText.trim()) {
      onEdit(todo.id, editedText.trim());
      setIsEditing(false);
    }
  };

  // Format the due date for display
  const formattedDueDate = todo.dueDate 
    ? new Date(todo.dueDate).toLocaleDateString(undefined, { 
        month: 'short', 
        day: 'numeric',
        year: 'numeric'
      }) 
    : null;

  // Determine if the todo is overdue
  const isOverdue = todo.dueDate 
    ? new Date(todo.dueDate).setHours(23, 59, 59, 999) < Date.now() && !todo.completed
    : false;

  // Get priority color
  const priorityColors = {
    low: "bg-green-100 text-green-800",
    medium: "bg-blue-100 text-blue-800",
    high: "bg-red-100 text-red-800"
  };

  return (
    <li className="border-b border-gray-200 last:border-b-0">
      <div className="py-3 flex items-start">
        <input
          type="checkbox"
          checked={todo.completed}
          onChange={() => onToggle(todo.id)}
          className="mt-1 h-5 w-5 text-blue-600 rounded focus:ring-blue-500"
          data-testid={`todo-checkbox-${todo.id}`}
        />
        
        <div className="ml-3 flex-grow">
          {isEditing ? (
            <form onSubmit={handleEditSubmit} className="flex items-center">
              <input
                type="text"
                value={editedText}
                onChange={(e) => setEditedText(e.target.value)}
                className="flex-grow px-2 py-1 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                autoFocus
                data-testid={`edit-input-${todo.id}`}
              />
              <button
                type="submit"
                className="ml-2 bg-green-500 hover:bg-green-600 text-white font-medium py-1 px-3 text-sm rounded-md transition-colors"
                data-testid={`save-edit-${todo.id}`}
              >
                Save
              </button>
              <button
                type="button"
                onClick={() => {
                  setIsEditing(false);
                  setEditedText(todo.text);
                }}
                className="ml-2 bg-gray-300 hover:bg-gray-400 text-gray-700 font-medium py-1 px-3 text-sm rounded-md transition-colors"
              >
                Cancel
              </button>
            </form>
          ) : (
            <div>
              <div className="flex items-center">
                <span 
                  className={`text-md ${todo.completed ? 'line-through text-gray-500' : 'text-gray-800'}`}
                  data-testid={`todo-text-${todo.id}`}
                >
                  {todo.text}
                </span>
                
                <div className="flex ml-2 space-x-1">
                  <span className={`text-xs px-2 py-0.5 rounded-full ${priorityColors[todo.priority]}`}>
                    {todo.priority}
                  </span>
                  
                  {formattedDueDate && (
                    <span className={`text-xs px-2 py-0.5 rounded-full ${
                      isOverdue 
                        ? 'bg-red-100 text-red-800' 
                        : 'bg-gray-100 text-gray-800'
                    }`}>
                      {formattedDueDate}
                    </span>
                  )}
                </div>
              </div>
              
              {isExpanded && (
                <div className="mt-2 flex flex-col md:flex-row space-y-2 md:space-y-0 md:space-x-4">
                  <div className="flex items-center space-x-2">
                    <label htmlFor={`priority-${todo.id}`} className="text-sm text-gray-600">
                      Priority:
                    </label>
                    <select
                      id={`priority-${todo.id}`}
                      value={todo.priority}
                      onChange={(e) => onUpdatePriority(todo.id, e.target.value as Todo["priority"])}
                      className="px-2 py-1 text-sm border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                    >
                      <option value="low">Low</option>
                      <option value="medium">Medium</option>
                      <option value="high">High</option>
                    </select>
                  </div>
                  
                  <div className="flex items-center space-x-2">
                    <label htmlFor={`dueDate-${todo.id}`} className="text-sm text-gray-600">
                      Due date:
                    </label>
                    <input
                      type="date"
                      id={`dueDate-${todo.id}`}
                      value={todo.dueDate || ""}
                      onChange={(e) => onUpdateDueDate(todo.id, e.target.value || undefined)}
                      className="px-2 py-1 text-sm border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                    {todo.dueDate && (
                      <button
                        onClick={() => onUpdateDueDate(todo.id, undefined)}
                        className="text-xs px-1 text-gray-500 hover:text-red-500"
                      >
                        Clear
                      </button>
                    )}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
        
        <div className="ml-2 flex items-center space-x-1">
          <button
            onClick={() => setIsExpanded(!isExpanded)}
            className="text-gray-400 hover:text-gray-600 p-1"
            title={isExpanded ? "Hide details" : "Show details"}
          >
            {isExpanded ? "▲" : "▼"}
          </button>
          
          {!isEditing && (
            <button
              onClick={() => {
                setIsEditing(true);
                setEditedText(todo.text);
              }}
              className="text-gray-400 hover:text-gray-600 p-1"
              title="Edit todo"
              data-testid={`edit-button-${todo.id}`}
            >
              ✎
            </button>
          )}
          
          <button
            onClick={() => onDelete(todo.id)}
            className="text-gray-400 hover:text-red-500 p-1"
            title="Delete todo"
            data-testid={`delete-button-${todo.id}`}
          >
            ×
          </button>
        </div>
      </div>
    </li>
  );
};

const TodoList: React.FC = () => {
  const {
    getFilteredAndSortedTodos,
    toggleTodo,
    deleteTodo,
    updateTodoText,
    updateTodoPriority,
    updateTodoDueDate,
    clearCompleted,
    deleteAll,
    getRemainingCount,
    getCompletedCount,
    currentFilter,
    setFilter,
    currentSort,
    setSort,
    sortDirection,
    toggleSortDirection,
  } = useTodoStore();

  const todos = getFilteredAndSortedTodos();
  const remainingCount = getRemainingCount();
  const completedCount = getCompletedCount();

  const filterOptions: { value: FilterOption; label: string }[] = [
    { value: "all", label: "All" },
    { value: "active", label: "Active" },
    { value: "completed", label: "Completed" },
  ];

  const sortOptions: { value: SortOption; label: string }[] = [
    { value: "createdAt", label: "Creation Date" },
    { value: "priority", label: "Priority" },
    { value: "dueDate", label: "Due Date" },
  ];

  if (todos.length === 0) {
    return (
      <div className="bg-white rounded-lg shadow-sm p-4">
        <p className="text-gray-500 text-center py-8">
          {currentFilter === "all" 
            ? "No todos yet. Add one above!" 
            : `No ${currentFilter} todos.`}
        </p>
      </div>
    );
  }

  return (
    <div className="todo-list bg-white rounded-lg shadow-sm overflow-hidden">
      <div className="p-3 bg-gray-50 border-b border-gray-200 flex flex-col sm:flex-row justify-between items-start sm:items-center gap-2">
        <div>
          <span className="text-sm text-gray-600">
            {remainingCount} item{remainingCount !== 1 ? "s" : ""} left
          </span>
          {completedCount > 0 && (
            <button
              onClick={clearCompleted}
              className="ml-4 text-sm text-blue-500 hover:text-blue-700"
              data-testid="clear-completed-button"
            >
              Clear completed
            </button>
          )}
          {todos.length > 5 && (
            <button
              onClick={deleteAll}
              className="ml-4 text-sm text-red-500 hover:text-red-700"
              data-testid="delete-all-button"
            >
              Delete all
            </button>
          )}
        </div>
        
        <div className="flex flex-wrap gap-2">
          <div className="flex items-center">
            <span className="text-sm text-gray-600 mr-2">Filter:</span>
            <div className="flex rounded-md overflow-hidden border border-gray-300">
              {filterOptions.map((option) => (
                <button
                  key={option.value}
                  onClick={() => setFilter(option.value)}
                  className={`px-2 py-1 text-sm ${
                    currentFilter === option.value
                      ? "bg-blue-500 text-white"
                      : "bg-white text-gray-700 hover:bg-gray-100"
                  }`}
                >
                  {option.label}
                </button>
              ))}
            </div>
          </div>
          
          <div className="flex items-center">
            <span className="text-sm text-gray-600 mr-2">Sort:</span>
            <select
              value={currentSort}
              onChange={(e) => setSort(e.target.value as SortOption)}
              className="text-sm border border-gray-300 rounded-md px-2 py-1 focus:outline-none focus:ring-2 focus:ring-blue-500"
            >
              {sortOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
            <button
              onClick={toggleSortDirection}
              className="ml-1 px-2 py-1 text-sm bg-white border border-gray-300 rounded-md hover:bg-gray-100"
              title={sortDirection === "asc" ? "Ascending" : "Descending"}
            >
              {sortDirection === "asc" ? "↑" : "↓"}
            </button>
          </div>
        </div>
      </div>
      
      <ul className="divide-y divide-gray-200 px-4">
        {todos.map((todo) => (
          <TodoItem
            key={todo.id}
            todo={todo}
            onToggle={toggleTodo}
            onDelete={deleteTodo}
            onEdit={updateTodoText}
            onUpdatePriority={updateTodoPriority}
            onUpdateDueDate={updateTodoDueDate}
          />
        ))}
      </ul>
    </div>
  );
};

export default TodoList;
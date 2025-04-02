import React, { useState } from "react";
import { useTodoStore } from "../stores/todoStore";

const AddTodo: React.FC = () => {
  const [text, setText] = useState("");
  const [priority, setPriority] = useState<"low" | "medium" | "high">("medium");
  const [dueDate, setDueDate] = useState<string>("");
  const [isExpanded, setIsExpanded] = useState(false);
  const { addTodo } = useTodoStore();

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (text.trim()) {
      // Add the todo with the current settings
      addTodo(text.trim(), priority, dueDate || undefined);
      
      // Reset the form
      setText("");
      setPriority("medium");
      setDueDate("");
      setIsExpanded(false);
    }
  };

  return (
    <form onSubmit={handleSubmit} className="add-todo bg-white rounded-lg shadow-sm p-4 mb-4">
      <div className="flex flex-col space-y-3">
        <div className="flex flex-row space-x-2">
          <input
            type="text"
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="What needs to be done?"
            className="flex-grow px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
            data-testid="add-todo-input"
          />
          <button 
            type="submit" 
            className="bg-blue-500 hover:bg-blue-600 text-white font-medium py-2 px-4 rounded-md transition-colors focus:outline-none focus:ring-2 focus:ring-blue-700"
            data-testid="add-todo-button"
          >
            Add
          </button>
          <button
            type="button"
            onClick={() => setIsExpanded(!isExpanded)}
            className="bg-gray-200 hover:bg-gray-300 text-gray-700 font-medium py-2 px-3 rounded-md transition-colors focus:outline-none focus:ring-2 focus:ring-gray-400"
          >
            {isExpanded ? "−" : "+"}
          </button>
        </div>
        
        {isExpanded && (
          <div className="flex flex-col md:flex-row space-y-3 md:space-y-0 md:space-x-4 pt-2">
            <div className="flex flex-col space-y-1">
              <label htmlFor="priority" className="text-sm text-gray-600">
                Priority
              </label>
              <select
                id="priority"
                value={priority}
                onChange={(e) => setPriority(e.target.value as "low" | "medium" | "high")}
                className="px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                data-testid="priority-select"
              >
                <option value="low">Low</option>
                <option value="medium">Medium</option>
                <option value="high">High</option>
              </select>
            </div>
            
            <div className="flex flex-col space-y-1">
              <label htmlFor="dueDate" className="text-sm text-gray-600">
                Due Date (optional)
              </label>
              <input
                type="date"
                id="dueDate"
                value={dueDate}
                onChange={(e) => setDueDate(e.target.value)}
                className="px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                data-testid="due-date-input"
              />
            </div>
          </div>
        )}
      </div>
    </form>
  );
};

export default AddTodo;
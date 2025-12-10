import React from 'react';
import { type Todo, useTodoStore } from '../stores/todoStore';

const TodoList = () => {
  const { todos, deleteTodo, toggleTodo } = useTodoStore();

  if (todos.length === 0) {
    return <p>No todos yet. Add one above!</p>;
  }

  return (
    <ul className="todo-list">
      {todos.map((todo: Todo) => (
        <li
          key={todo.id}
          className={`todo-item ${todo.completed ? 'completed' : ''}`}
        >
          <input
            type="checkbox"
            checked={todo.completed}
            onChange={() => toggleTodo(todo.id)}
          />
          <span className="todo-text">{todo.text}</span>
          <button className="delete-btn" onClick={() => deleteTodo(todo.id)}>
            Delete
          </button>
        </li>
      ))}
    </ul>
  );
};

export default TodoList;

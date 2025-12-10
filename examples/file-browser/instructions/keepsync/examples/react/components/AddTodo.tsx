import type React from 'react';
import { useState } from 'react';
import { useTodoStore } from '../stores/todoStore';

const AddTodo = () => {
  const [text, setText] = useState('');
  const { addTodo } = useTodoStore();

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (text.trim()) {
      addTodo(text.trim());
      setText('');
    }
  };

  return (
    <form onSubmit={handleSubmit} className="add-todo">
      <input
        type="text"
        value={text}
        onChange={e => setText(e.target.value)}
        placeholder="What needs to be done?"
      />
      <button type="submit">Add Todo</button>
    </form>
  );
};

export default AddTodo;

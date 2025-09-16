import React from 'react';
import { Compiler } from '../components/Compiler';

/**
 * Test page for the simplified TypeScript compiler
 */
export const CompilerTest: React.FC = () => {
  const exampleCode = `// No imports needed! React and hooks are automatically available

export default function TodoList() {
  const [todos, setTodos] = useState([
    { id: 1, text: 'Learn TypeScript', done: false },
    { id: 2, text: 'Build cool stuff', done: false }
  ]);
  const [inputText, setInputText] = useState('');

  const addTodo = () => {
    if (inputText.trim()) {
      setTodos([...todos, {
        id: Date.now(),
        text: inputText,
        done: false
      }]);
      setInputText('');
    }
  };

  const toggleTodo = (id) => {
    setTodos(todos.map(todo =>
      todo.id === id ? { ...todo, done: !todo.done } : todo
    ));
  };

  return (
    <div style={{ padding: '20px' }}>
      <h2>Simple Todo App</h2>

      <div style={{ marginBottom: '20px' }}>
        <input
          type="text"
          value={inputText}
          onChange={(e) => setInputText(e.target.value)}
          onKeyPress={(e) => e.key === 'Enter' && addTodo()}
          placeholder="Add a todo..."
          style={{
            padding: '8px',
            marginRight: '8px',
            borderRadius: '4px',
            border: '1px solid #ccc'
          }}
        />
        <button 
          onClick={addTodo}
          style={{
            padding: '8px 16px',
            backgroundColor: '#4CAF50',
            color: 'white',
            border: 'none',
            borderRadius: '4px',
            cursor: 'pointer'
          }}
        >
          Add
        </button>
      </div>

      <ul style={{ listStyle: 'none', padding: 0 }}>
        {todos.map(todo => (
          <li
            key={todo.id}
            onClick={() => toggleTodo(todo.id)}
            style={{
              padding: '8px',
              margin: '4px 0',
              backgroundColor: todo.done ? '#e8f5e9' : '#f5f5f5',
              borderRadius: '4px',
              cursor: 'pointer',
              textDecoration: todo.done ? 'line-through' : 'none',
              color: todo.done ? '#888' : '#333'
            }}
          >
            {todo.text}
          </li>
        ))}
      </ul>
    </div>
  );
}`;

  return (
    <div className="min-h-screen bg-gray-100 p-8">
      <div className="max-w-6xl mx-auto">
        <h1 className="text-3xl font-bold text-gray-800 mb-2">
          TypeScript Compiler
        </h1>
        <p className="text-gray-600 mb-8">
          Write TypeScript/React code without imports. All React hooks and
          utilities are automatically available.
        </p>

        <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 mb-6">
          <h3 className="font-semibold text-blue-900 mb-2">
            Available without imports:
          </h3>
          <code className="text-sm text-blue-800">
            React, useState, useEffect, useCallback, useMemo, useRef,
            useReducer, useContext, Fragment
          </code>
        </div>

        <Compiler
          initialCode={exampleCode}
          height="500px"
          onCompiled={result => {
            console.log('Compiled:', result);
          }}
        />
      </div>
    </div>
  );
};

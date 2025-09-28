import React, { useState, useEffect } from 'react';
import { Plus, Folder, Lightbulb, Code, Eye } from 'lucide-react';
import { ComponentBrowser } from './ComponentBrowser';
import { ComponentEditor } from './ComponentEditor';
import { ComponentPreview } from './ComponentPreview';
import { AvailableComponentsPanel } from './AvailableComponentsPanel';
import { componentRegistry } from './ComponentRegistry';
import { useVFSComponent } from './hooks/useVFSComponent';
import { useComponentWatcher } from './hooks/useComponentWatcher';
import { getVFSService } from '../services/vfs-service';
import AgentChat from '../views/AgentChat';

const DEFAULT_COMPONENT_TEMPLATE = `export default function MyComponent() {
  const [count, setCount] = useState(0);

  return (
    <div style={{ padding: '20px', textAlign: 'center' }}>
      <h2>Hello from MyComponent!</h2>
      <p>Count: {count}</p>
      <button 
        onClick={() => setCount(count + 1)}
        style={{
          padding: '8px 16px',
          backgroundColor: '#4CAF50',
          color: 'white',
          border: 'none',
          borderRadius: '4px',
          cursor: 'pointer'
        }}
      >
        Increment
      </button>
    </div>
  );
}`;

const TEMPLATE_OPTIONS = [
  {
    name: 'Counter Component',
    description: 'Simple counter with useState',
    code: DEFAULT_COMPONENT_TEMPLATE,
  },
  {
    name: 'Todo List',
    description: 'Interactive todo list component',
    code: `export default function TodoList() {
  const [todos, setTodos] = useState([
    { id: 1, text: 'Learn React', done: false },
    { id: 2, text: 'Build something awesome', done: false }
  ]);
  const [newTodo, setNewTodo] = useState('');

  const addTodo = () => {
    if (newTodo.trim()) {
      setTodos([...todos, {
        id: Date.now(),
        text: newTodo,
        done: false
      }]);
      setNewTodo('');
    }
  };

  const toggleTodo = (id) => {
    setTodos(todos.map(todo =>
      todo.id === id ? { ...todo, done: !todo.done } : todo
    ));
  };

  return (
    <div style={{ padding: '20px' }}>
      <h2>Todo List</h2>

      <div style={{ marginBottom: '20px' }}>
        <input
          type="text"
          value={newTodo}
          onChange={(e) => setNewTodo(e.target.value)}
          onKeyPress={(e) => e.key === 'Enter' && addTodo()}
          placeholder="Add a new todo..."
          style={{ padding: '8px', marginRight: '8px', width: '200px' }}
        />
        <button onClick={addTodo} style={{ padding: '8px 16px' }}>
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
              textDecoration: todo.done ? 'line-through' : 'none'
            }}
          >
            {todo.text}
          </li>
        ))}
      </ul>
    </div>
  );
}`,
  },
  {
    name: 'Form Component',
    description: 'Form with validation',
    code: `export default function ContactForm() {
  const [formData, setFormData] = useState({
    name: '',
    email: '',
    message: ''
  });
  const [errors, setErrors] = useState({});

  const handleChange = (e) => {
    setFormData({
      ...formData,
      [e.target.name]: e.target.value
    });
  };

  const validate = () => {
    const newErrors = {};
    if (!formData.name.trim()) newErrors.name = 'Name is required';
    if (!formData.email.trim()) newErrors.email = 'Email is required';
    if (!formData.message.trim()) newErrors.message = 'Message is required';

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  };

  const handleSubmit = (e) => {
    e.preventDefault();
    if (validate()) {
      alert('Form submitted!');
      setFormData({ name: '', email: '', message: '' });
    }
  };

  return (
    <div style={{ padding: '20px', maxWidth: '400px' }}>
      <h2>Contact Form</h2>

      <form onSubmit={handleSubmit}>
        <div style={{ marginBottom: '15px' }}>
          <input
            type="text"
            name="name"
            placeholder="Your Name"
            value={formData.name}
            onChange={handleChange}
            style={{ 
              width: '100%', 
              padding: '8px', 
              border: errors.name ? '1px solid red' : '1px solid #ccc' 
            }}
          />
          {errors.name && <div style={{ color: 'red', fontSize: '12px' }}>{errors.name}</div>}
        </div>

        <div style={{ marginBottom: '15px' }}>
          <input
            type="email"
            name="email"
            placeholder="Your Email"
            value={formData.email}
            onChange={handleChange}
            style={{ 
              width: '100%', 
              padding: '8px', 
              border: errors.email ? '1px solid red' : '1px solid #ccc' 
            }}
          />
          {errors.email && <div style={{ color: 'red', fontSize: '12px' }}>{errors.email}</div>}
        </div>

        <div style={{ marginBottom: '15px' }}>
          <textarea
            name="message"
            placeholder="Your Message"
            value={formData.message}
            onChange={handleChange}
            rows={4}
            style={{ 
              width: '100%', 
              padding: '8px', 
              border: errors.message ? '1px solid red' : '1px solid #ccc' 
            }}
          />
          {errors.message && <div style={{ color: 'red', fontSize: '12px' }}>{errors.message}</div>}
        </div>

        <button type="submit" style={{ 
          padding: '10px 20px', 
          backgroundColor: '#007bff', 
          color: 'white', 
          border: 'none', 
          borderRadius: '4px' 
        }}>
          Send Message
        </button>
      </form>
    </div>
  );
}`,
  },
];

export const ComponentManager: React.FC = () => {
  const [selectedComponentId, setSelectedComponentId] = useState<string | null>(
    null
  );
  const [showNewComponentDialog, setShowNewComponentDialog] = useState(false);
  const [newComponentName, setNewComponentName] = useState('');
  const [selectedTemplate, setSelectedTemplate] = useState(0);
  const [activeTab, setActiveTab] = useState<'edit' | 'preview'>('preview');

  const { createFile } = useVFSComponent(null);
  const { watchComponent, unwatchComponent } = useComponentWatcher();
  const vfs = getVFSService();

  useEffect(() => {
    const loadTypeScript = async () => {
      if (!(window as any).ts) {
        const script = document.createElement('script');
        script.src = '/typescript.js';
        script.onload = () => {};
        document.head.appendChild(script);
      }
    };
    loadTypeScript();
  }, []);

  const createNewComponent = async () => {
    if (!newComponentName.trim()) return;

    try {
      const componentName = newComponentName.trim();
      const template = TEMPLATE_OPTIONS[selectedTemplate];
      const componentCode = template.code.replace('MyComponent', componentName);

      const componentId = componentRegistry.createComponent(componentName);
      const component = componentRegistry.getComponent(componentId);

      if (component) {
        await createFile(component.metadata.filePath, componentCode);
        await watchComponent(componentId, component.metadata.filePath);

        setSelectedComponentId(componentId);
        setShowNewComponentDialog(false);
        setNewComponentName('');
        setSelectedTemplate(0);
      }
    } catch (error) {
      console.error('Failed to create component:', error);
      alert('Failed to create component. Please try again.');
    }
  };

  const deleteComponent = async (componentId: string) => {
    try {
      const component = componentRegistry.getComponent(componentId);
      if (component) {
        // Stop watching the component file
        await unwatchComponent(componentId);

        // Delete the component file from VFS
        try {
          await vfs.deleteFile(component.metadata.filePath);
        } catch (fileError) {
          console.warn(
            `Failed to delete component file ${component.metadata.filePath}:`,
            fileError
          );
        }
      }

      // Remove component from registry
      componentRegistry.deleteComponent(componentId);

      // Clear selection if this was the selected component
      if (selectedComponentId === componentId) {
        setSelectedComponentId(null);
      }
    } catch (error) {
      console.error('Failed to delete component:', error);
      alert('Failed to delete component. Please try again.');
    }
  };

  return (
    <div className="flex h-full bg-gray-100 overflow-hidden">
      {/* Component Browser Sidebar */}
      <ComponentBrowser
        selectedComponentId={selectedComponentId}
        onSelectComponent={setSelectedComponentId}
        onDeleteComponent={deleteComponent}
        onCreateComponent={() => setShowNewComponentDialog(true)}
      />

      {/* Main Content Area */}
      <div className="flex-1 flex flex-col h-full overflow-hidden">
        {/* Header */}
        <div className="bg-white border-b border-gray-200 px-6 py-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-4">
              <Folder className="w-5 h-5 text-blue-500" />
              <h2 className="text-lg font-semibold text-gray-800">
                Component Editor
              </h2>
              {selectedComponentId && (
                <span className="text-sm text-gray-500">
                  {componentRegistry.getComponent(selectedComponentId)?.metadata.filePath}
                </span>
              )}
            </div>
          </div>
        </div>

        {/* Editor and Preview with Tabs */}
        <div className="flex-1 flex flex-col overflow-hidden">
          {/* Tab Bar */}
          <div className="bg-white border-b border-gray-200 px-6">
            <div className="flex gap-1">
              <button
              type="button"
                onClick={() => setActiveTab('preview')}
                className={`px-4 py-2 font-medium text-sm border-b-2 transition-colors ${
                  activeTab === 'preview'
                    ? 'border-blue-500 text-blue-600'
                    : 'border-transparent text-gray-500 hover:text-gray-700'
                }`}
              >
                <div className="flex items-center gap-2">
                  <Eye className="w-4 h-4" />
                  Preview
                </div>
              </button>
              <button
              type="button"
                onClick={() => setActiveTab('edit')}
                className={`px-4 py-2 font-medium text-sm border-b-2 transition-colors ${
                  activeTab === 'edit'
                    ? 'border-blue-500 text-blue-600'
                    : 'border-transparent text-gray-500 hover:text-gray-700'
                }`}
              >
                <div className="flex items-center gap-2">
                  <Code className="w-4 h-4" />
                  Edit
                </div>
              </button>
            </div>
          </div>

          {/* Tab Content */}
          <div className="flex-1 flex overflow-hidden">
            {activeTab === 'edit' ? (
              <div className="flex-1 p-6 overflow-auto">
                <ComponentEditor
                  componentId={selectedComponentId}
                  height="calc(100vh - 250px)"
                />
              </div>
            ) : (
              <div className="flex-1 p-6 overflow-auto">
                <ComponentPreview
                  componentId={selectedComponentId}
                  className="h-full"
                />
              </div>
            )}
          </div>
        </div>
      </div>

      {/* New Component Dialog */}
      {showNewComponentDialog && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-white rounded-lg p-6 w-full max-w-md mx-4">
            <div className="flex items-center gap-3 mb-4">
              <Lightbulb className="w-6 h-6 text-yellow-500" />
              <h2 className="text-lg font-semibold">Create New Component</h2>
            </div>

            <div className="space-y-4">
              <div>
                <label 
                className="block text-sm font-medium text-gray-700 mb-2">
                  Component Name
                </label>
                <input
                  type="text"
                  value={newComponentName}
                  onChange={e => setNewComponentName(e.target.value)}
                  placeholder="e.g., MyAwesomeComponent"
                  className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                />
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-700 mb-2">
                  Template
                </label>
                <select
                  value={selectedTemplate}
                  onChange={e => setSelectedTemplate(Number(e.target.value))}
                  className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                >
                  {TEMPLATE_OPTIONS.map((template, index) => (
                    <option key={index} value={index}>
                      {template.name} - {template.description}
                    </option>
                  ))}
                </select>
              </div>
            </div>

            <div className="flex gap-3 mt-6">
              <button
                onClick={() => setShowNewComponentDialog(false)}
                className="flex-1 px-4 py-2 border border-gray-300 text-gray-700 rounded-lg hover:bg-gray-50 transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={createNewComponent}
                disabled={!newComponentName.trim()}
                className="flex-1 px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              >
                Create
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

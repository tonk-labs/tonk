import React, { useState, useEffect } from 'react';
import {
  Search,
  FileText,
  AlertCircle,
  CheckCircle,
  Clock,
  Trash2,
  Plus,
} from 'lucide-react';
import {
  componentRegistry,
  ProxiedComponent,
  ComponentMetadata,
} from './ComponentRegistry';

interface ComponentBrowserProps {
  selectedComponentId: string | null;
  onSelectComponent: (componentId: string | null) => void;
  onDeleteComponent: (componentId: string) => void;
  onCreateComponent: () => void;
}

export const ComponentBrowser: React.FC<ComponentBrowserProps> = ({
  selectedComponentId,
  onSelectComponent,
  onDeleteComponent,
  onCreateComponent,
}) => {
  const [components, setComponents] = useState<ProxiedComponent[]>([]);
  const [searchTerm, setSearchTerm] = useState('');
  const [sortBy, setSortBy] = useState<'name' | 'modified' | 'status'>(
    'modified'
  );

  useEffect(() => {
    const updateComponents = () => {
      setComponents(componentRegistry.getAllComponents());
    };

    updateComponents();

    const interval = setInterval(updateComponents, 1000);
    return () => clearInterval(interval);
  }, []);

  const filteredComponents = components
    .filter(
      comp =>
        comp.metadata.name.toLowerCase().includes(searchTerm.toLowerCase()) ||
        comp.metadata.filePath.toLowerCase().includes(searchTerm.toLowerCase())
    )
    .sort((a, b) => {
      switch (sortBy) {
        case 'name':
          return a.metadata.name.localeCompare(b.metadata.name);
        case 'modified':
          return b.metadata.modified.getTime() - a.metadata.modified.getTime();
        case 'status':
          const statusOrder = { error: 0, loading: 1, success: 2 };
          return (
            statusOrder[a.metadata.status] - statusOrder[b.metadata.status]
          );
        default:
          return 0;
      }
    });

  const getStatusIcon = (status: ComponentMetadata['status']) => {
    switch (status) {
      case 'success':
        return <CheckCircle className="w-4 h-4 text-green-500" />;
      case 'error':
        return <AlertCircle className="w-4 h-4 text-red-500" />;
      case 'loading':
        return <Clock className="w-4 h-4 text-blue-500" />;
      default:
        return <FileText className="w-4 h-4 text-gray-400" />;
    }
  };

  const getStatusColor = (status: ComponentMetadata['status']) => {
    switch (status) {
      case 'success':
        return 'border-green-200 bg-green-50';
      case 'error':
        return 'border-red-200 bg-red-50';
      case 'loading':
        return 'border-blue-200 bg-blue-50';
      default:
        return 'border-gray-200 bg-gray-50';
    }
  };

  const formatTime = (date: Date) => {
    const now = new Date();
    const diff = now.getTime() - date.getTime();
    const minutes = Math.floor(diff / 60000);

    if (minutes === 0) return 'Just now';
    if (minutes === 1) return '1 min ago';
    if (minutes < 60) return `${minutes} mins ago`;

    const hours = Math.floor(minutes / 60);
    if (hours === 1) return '1 hour ago';
    if (hours < 24) return `${hours} hours ago`;

    const days = Math.floor(hours / 24);
    if (days === 1) return '1 day ago';
    return `${days} days ago`;
  };

  const handleDeleteComponent = (e: React.MouseEvent, componentId: string) => {
    e.stopPropagation();
    if (confirm('Are you sure you want to delete this component?')) {
      onDeleteComponent(componentId);
    }
  };

  return (
    <div className="w-64 bg-white border-r border-gray-200 flex flex-col h-full">
      <div className="p-4 border-b border-gray-200">
        <div className="flex items-center justify-between mb-3">
          <h3 className="font-semibold text-gray-800">Components</h3>
          <button
            onClick={onCreateComponent}
            className="p-1 hover:bg-gray-100 rounded transition-colors"
            title="Create new component"
          >
            <Plus className="w-4 h-4 text-gray-600" />
          </button>
        </div>
        <div className="relative mb-3">
          <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 text-gray-400 w-4 h-4" />
          <input
            type="text"
            placeholder="Search components..."
            value={searchTerm}
            onChange={e => setSearchTerm(e.target.value)}
            className="w-full pl-10 pr-4 py-2 border border-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent text-sm"
          />
        </div>

        <select
          value={sortBy}
          onChange={e =>
            setSortBy(e.target.value as 'name' | 'modified' | 'status')
          }
          className="w-full px-3 py-2 border border-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 text-sm"
        >
          <option value="modified">Sort by Modified</option>
          <option value="name">Sort by Name</option>
          <option value="status">Sort by Status</option>
        </select>
      </div>

      <div className="flex-1 overflow-y-auto">
        {filteredComponents.length === 0 ? (
          <div className="p-6 text-center text-gray-500">
            <FileText className="w-12 h-12 mx-auto mb-3 text-gray-300" />
            <p className="text-sm">
              {searchTerm
                ? 'No components match your search'
                : 'No components yet'}
            </p>
          </div>
        ) : (
          <div className="p-2">
            {filteredComponents.map(component => (
              <div
                key={component.id}
                onClick={() => onSelectComponent(component.id)}
                className={`
                  p-3 mb-2 rounded-lg border cursor-pointer transition-all duration-200 group
                  ${
                    selectedComponentId === component.id
                      ? 'border-blue-500 bg-blue-50 shadow-sm'
                      : `hover:shadow-sm ${getStatusColor(component.metadata.status)}`
                  }
                `}
              >
                <div className="flex items-start justify-between">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 mb-1">
                      {getStatusIcon(component.metadata.status)}
                      <h3 className="font-medium text-gray-900 text-sm truncate">
                        {component.metadata.name}
                      </h3>
                    </div>

                    <p className="text-xs text-gray-500 truncate mb-1">
                      {component.metadata.filePath}
                    </p>

                    <p className="text-xs text-gray-400">
                      {formatTime(component.metadata.modified)}
                    </p>

                    {component.metadata.status === 'error' &&
                      component.metadata.error && (
                        <p className="text-xs text-red-600 mt-1 truncate">
                          {component.metadata.error}
                        </p>
                      )}
                  </div>

                  <button
                    onClick={e => handleDeleteComponent(e, component.id)}
                    className="opacity-0 group-hover:opacity-100 transition-opacity duration-200 p-1 hover:bg-red-100 rounded text-red-500"
                    title="Delete component"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};

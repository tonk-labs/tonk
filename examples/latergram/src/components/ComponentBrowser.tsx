import React, { useState, useEffect } from 'react';
import {
  FileText,
  AlertCircle,
  CheckCircle,
  Clock,
  File,
} from 'lucide-react';
import { EditorSidebar, SearchInput, SidebarItem, EmptyState } from './shared';
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

  // Removed handleDeleteComponent as it's now handled by SidebarItem

  return (
    <EditorSidebar
      title="Components"
      onCreateClick={onCreateComponent}
    >
      <div className="p-4 border-b border-gray-200">
        <div className="mb-3">
          <SearchInput
            value={searchTerm}
            onChange={setSearchTerm}
            placeholder="Search components..."
            focusColor="blue"
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
          <EmptyState
            icon={<FileText className="w-12 h-12 text-gray-300" />}
            title={searchTerm ? 'No components match your search' : 'No components yet'}
          />
        ) : (
          <div className="p-2">
            {filteredComponents.map(component => (
              <SidebarItem
                key={component.id}
                selected={selectedComponentId === component.id}
                onClick={() => onSelectComponent(component.id)}
                onDelete={() => onDeleteComponent(component.id)}
                icon={getStatusIcon(component.metadata.status)}
                title={component.metadata.name}
                subtitle={component.metadata.filePath}
                status={component.metadata.status}
                error={component.metadata.error}
                color="blue"
              >
                <p className="text-xs text-gray-400 mt-1 ml-6">
                  {formatTime(component.metadata.modified)}
                </p>
              </SidebarItem>
            ))}
          </div>
        )}
      </div>
    </EditorSidebar>
  );
};

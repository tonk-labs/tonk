import React, { useState, useEffect } from 'react';
import {
  Plus,
  Database,
  Search,
  Trash2,
  Circle,
  AlertCircle,
} from 'lucide-react';
import { storeRegistry, ProxiedStore } from './StoreRegistry';
import { STORE_TEMPLATES } from './StoreTemplates';

interface StoreBrowserProps {
  selectedStoreId?: string | null;
  onStoreSelect?: (storeId: string) => void;
  onCreateStore?: (templateName: string, storeName: string) => void;
  onDeleteStore?: (storeId: string) => void;
}

export const StoreBrowser: React.FC<StoreBrowserProps> = ({
  selectedStoreId,
  onStoreSelect,
  onCreateStore,
  onDeleteStore,
}) => {
  const [stores, setStores] = useState<ProxiedStore[]>([]);
  const [searchTerm, setSearchTerm] = useState('');
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [newStoreName, setNewStoreName] = useState('');
  const [selectedTemplate, setSelectedTemplate] = useState(
    STORE_TEMPLATES[0].name
  );
  // Update stores list when registry changes
  useEffect(() => {
    const updateStores = () => {
      setStores(storeRegistry.getAllStores());
    };

    updateStores();
    const unsubscribe = storeRegistry.onContextUpdate(updateStores);
    return unsubscribe;
  }, []);

  // Filter stores based on search
  const filteredStores = stores.filter(store => {
    const matchesSearch = store.metadata.name
      .toLowerCase()
      .includes(searchTerm.toLowerCase());
    return matchesSearch;
  });

  const handleCreateStore = () => {
    if (!newStoreName.trim()) return;

    const templateName = selectedTemplate;
    onCreateStore?.(templateName, newStoreName.trim());
    setNewStoreName('');
    setShowCreateModal(false);
  };

  const getStatusIcon = (status: ProxiedStore['metadata']['status']) => {
    switch (status) {
      case 'loading':
        return (
          <div className="w-3 h-3 border-2 border-blue-500 border-t-transparent rounded-full animate-spin" />
        );
      case 'success':
        return <Circle className="w-3 h-3 fill-green-500 text-green-500" />;
      case 'error':
        return <Circle className="w-3 h-3 fill-red-500 text-red-500" />;
      default:
        return <Circle className="w-3 h-3 text-gray-400" />;
    }
  };

  return (
    <div className="h-full bg-white rounded-lg border shadow-sm">
      {/* Header */}
      <div className="flex items-center justify-between p-4 border-b bg-gray-50">
        <div className="flex items-center gap-3">
          <Database className="w-5 h-5 text-purple-500" />
          <h3 className="font-semibold text-gray-800">Store Browser</h3>
          <span className="text-sm text-gray-500">
            ({stores.length} stores)
          </span>
        </div>
        <button
          onClick={() => setShowCreateModal(true)}
          className="flex items-center gap-2 px-3 py-1.5 bg-purple-600 text-white rounded-md hover:bg-purple-700 transition-colors text-sm"
        >
          <Plus className="w-4 h-4" />
          New Store
        </button>
      </div>

      {/* Search and Filter */}
      <div className="p-4 space-y-3 border-b">
        <div className="relative">
          <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-4 h-4 text-gray-400" />
          <input
            type="text"
            placeholder="Search stores..."
            value={searchTerm}
            onChange={e => setSearchTerm(e.target.value)}
            className="w-full pl-9 pr-4 py-2 border border-gray-200 rounded-md focus:outline-none focus:ring-2 focus:ring-purple-500 focus:border-transparent"
          />
        </div>
      </div>

      {/* Store List */}
      <div className="flex-1 overflow-y-auto">
        {Object.keys(filteredStores).length === 0 ? (
          <div className="flex flex-col items-center justify-center h-48 text-gray-500">
            <Database className="w-12 h-12 text-gray-300 mb-3" />
            <p className="text-sm">No stores found</p>
            <p className="text-xs text-gray-400 mt-1">
              Create your first store to get started
            </p>
          </div>
        ) : (
          <div className="p-2">
            <div className="space-y-1">
              {filteredStores.map(store => (
                <div
                  key={store.id}
                  onClick={() => onStoreSelect?.(store.id)}
                  className={`group flex items-center gap-3 p-2 rounded-md cursor-pointer transition-colors ${
                    selectedStoreId === store.id
                      ? 'bg-purple-50 border border-purple-200'
                      : 'hover:bg-gray-50'
                  }`}
                >
                  <div className="flex items-center gap-2 flex-1 min-w-0">
                    {getStatusIcon(store.metadata.status)}
                    <div className="flex-1 min-w-0">
                      <p className="text-sm font-medium text-gray-900 truncate">
                        {store.metadata.name}
                      </p>
                      <p className="text-xs text-gray-500 truncate">
                        {store.metadata.name}
                      </p>
                    </div>
                  </div>

                  {store.metadata.status === 'error' &&
                    store.metadata.error && (
                      <div title={store.metadata.error}>
                        <AlertCircle className="w-4 h-4 text-red-500" />
                      </div>
                    )}

                  <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                    <button
                      onClick={e => {
                        e.stopPropagation();
                        onDeleteStore?.(store.id);
                      }}
                      className="p-1 text-gray-400 hover:text-red-600 transition-colors"
                      title="Delete store"
                    >
                      <Trash2 className="w-3 h-3" />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Create Store Modal */}
      {showCreateModal && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-white rounded-lg p-6 w-96 max-w-full mx-4">
            <h3 className="text-lg font-semibold mb-4">Create New Store</h3>

            <div className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Store Name
                </label>
                <input
                  type="text"
                  value={newStoreName}
                  onChange={e => setNewStoreName(e.target.value)}
                  placeholder="e.g., UserStore, CartStore, etc."
                  className="w-full px-3 py-2 border border-gray-200 rounded-md focus:outline-none focus:ring-2 focus:ring-purple-500"
                  autoFocus
                />
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Template
                </label>
                <select
                  value={selectedTemplate}
                  onChange={e => setSelectedTemplate(e.target.value)}
                  className="w-full px-3 py-2 border border-gray-200 rounded-md focus:outline-none focus:ring-2 focus:ring-purple-500"
                >
                  {STORE_TEMPLATES.map(template => (
                    <option key={template.name} value={template.name}>
                      {template.name} - {template.description}
                    </option>
                  ))}
                </select>
              </div>
            </div>

            <div className="flex justify-end gap-3 mt-6">
              <button
                onClick={() => setShowCreateModal(false)}
                className="px-4 py-2 text-gray-600 hover:text-gray-800 transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleCreateStore}
                disabled={!newStoreName.trim()}
                className="px-4 py-2 bg-purple-600 text-white rounded-md hover:bg-purple-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              >
                Create Store
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

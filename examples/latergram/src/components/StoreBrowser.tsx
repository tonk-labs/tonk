import React, { useState, useEffect } from 'react';
import {
  Database,
  Circle,
  AlertCircle,
} from 'lucide-react';
import { EditorSidebar, SearchInput, SidebarItem, EmptyState } from './shared';
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
    <>
      <EditorSidebar
        title="Stores"
        onCreateClick={() => setShowCreateModal(true)}
      >
        <div className="p-4 border-b border-gray-200">
          <SearchInput
            value={searchTerm}
            onChange={setSearchTerm}
            placeholder="Search stores..."
            focusColor="purple"
          />
        </div>

        {/* Store List */}
        <div className="flex-1 overflow-y-auto">
          {filteredStores.length === 0 ? (
            <EmptyState
              icon={<Database className="w-12 h-12 text-gray-300" />}
              title="No stores found"
              subtitle="Create your first store to get started"
              actionText="Create store"
              onAction={() => setShowCreateModal(true)}
            />
          ) : (
            <div className="p-2">
              {filteredStores.map(store => (
                <SidebarItem
                  key={store.id}
                  selected={selectedStoreId === store.id}
                  onClick={() => onStoreSelect?.(store.id)}
                  onDelete={() => onDeleteStore?.(store.id)}
                  icon={getStatusIcon(store.metadata.status)}
                  title={store.metadata.name}
                  subtitle={store.metadata.filePath}
                  status={store.metadata.status}
                  error={store.metadata.error}
                  color="purple"
                />
              ))}
            </div>
          )}
        </div>
      </EditorSidebar>

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
    </>
  );
};

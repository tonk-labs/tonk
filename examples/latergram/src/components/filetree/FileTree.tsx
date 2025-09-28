import React, { useState, useEffect, useRef } from 'react';
import {
  UncontrolledTreeEnvironment,
  Tree,
  TreeItemIndex,
  InteractionMode,
  TreeItem
} from 'react-complex-tree';
import 'react-complex-tree/lib/style-modern.css';
import { VFSDataProvider, VFSTreeItemData } from './VFSDataProvider';
import { renderFileTreeNode } from './FileTreeNode';
import { Eye, EyeOff, RotateCcw, Search } from 'lucide-react';

interface FileTreeProps {
  rootPath?: string;
  onFileSelect?: (path: string) => void;
  onFileDelete?: (path: string) => void;
  showHidden?: boolean;
  className?: string;
}

export const FileTree: React.FC<FileTreeProps> = ({
  rootPath = '/',
  onFileSelect,
  onFileDelete,
  showHidden = false,
  className = ''
}) => {
  const [dataProvider] = useState(() => new VFSDataProvider());
  const [searchQuery, setSearchQuery] = useState('');
  const [showHiddenFiles, setShowHiddenFiles] = useState(showHidden);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedItems, setSelectedItems] = useState<TreeItemIndex[]>([]);
  // Expand common directories on first load
  const [expandedItems, setExpandedItems] = useState<TreeItemIndex[]>([
    'root',
    '/src',
    '/src/components',
    '/src/views',
    '/src/stores'
  ]);
  const [focusedItem, setFocusedItem] = useState<TreeItemIndex | undefined>();
  const [refreshKey, setRefreshKey] = useState(0);

  const treeRef = useRef<HTMLDivElement>(null);
  const expandedRef = useRef<TreeItemIndex[]>(expandedItems);
  const selectedRef = useRef<TreeItemIndex[]>(selectedItems);
  const focusedRef = useRef<TreeItemIndex | undefined>(focusedItem);

  // Keep refs in sync
  useEffect(() => {
    expandedRef.current = expandedItems;
  }, [expandedItems]);

  useEffect(() => {
    selectedRef.current = selectedItems;
  }, [selectedItems]);

  useEffect(() => {
    focusedRef.current = focusedItem;
  }, [focusedItem]);

  // Set up data change callback
  useEffect(() => {
    dataProvider.setOnDataChanged(() => {
      // Force refresh while preserving state
      setIsLoading(false);
      // Preserve the current state before refresh
      const currentExpanded = expandedRef.current;
      const currentSelected = selectedRef.current;
      const currentFocused = focusedRef.current;

      setRefreshKey(prev => prev + 1);

      // Restore state after refresh
      setTimeout(() => {
        setExpandedItems(currentExpanded);
        setSelectedItems(currentSelected);
        setFocusedItem(currentFocused);
      }, 0);
    });
  }, [dataProvider]);

  // Update showHidden in data provider when it changes
  useEffect(() => {
    dataProvider.setShowHidden(showHiddenFiles);
  }, [dataProvider, showHiddenFiles]);

  const handlePrimaryAction = (item: TreeItem<VFSTreeItemData>) => {
    if (item.data.type === 'file') {
      onFileSelect?.(item.data.path);
    }
  };

  const handleDelete = async (path: string) => {
    try {
      setIsLoading(true);
      setError(null);

      // Clean up state for deleted item and its children
      setSelectedItems(prev =>
        prev.filter(id => {
          const itemPath = id === 'root' ? '/' : id as string;
          return itemPath !== path && !itemPath.startsWith(path + '/');
        })
      );

      setExpandedItems(prev =>
        prev.filter(id => {
          const itemPath = id === 'root' ? '/' : id as string;
          return itemPath !== path && !itemPath.startsWith(path + '/');
        })
      );

      // Delete the item
      await dataProvider.deleteItem(path);
      onFileDelete?.(path);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to delete file';
      setError(errorMessage);
      console.error('Delete error:', err);
    } finally {
      setIsLoading(false);
    }
  };

  const handleCreateFile = async (parentPath: string) => {
    const fileName = window.prompt('Enter file name:');
    if (!fileName?.trim()) return;

    try {
      setIsLoading(true);
      setError(null);
      await dataProvider.createFile(parentPath, fileName.trim());
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to create file';
      setError(errorMessage);
      console.error('Create file error:', err);
    } finally {
      setIsLoading(false);
    }
  };

  const handleCreateDirectory = async (parentPath: string) => {
    const dirName = window.prompt('Enter directory name:');
    if (!dirName?.trim()) return;

    try {
      setIsLoading(true);
      setError(null);
      await dataProvider.createDirectory(parentPath, dirName.trim());
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to create directory';
      setError(errorMessage);
      console.error('Create directory error:', err);
    } finally {
      setIsLoading(false);
    }
  };

  const handleRefresh = () => {
    setIsLoading(true);
    setError(null);
    dataProvider.refresh();
    // Only reset to initial expanded state on manual refresh
    setExpandedItems([
      'root',
      '/src',
      '/src/components',
      '/src/views',
      '/src/stores'
    ]);
    setSelectedItems([]);
    setFocusedItem(undefined);
  };

  const filterItems = (items: TreeItemIndex[], query: string): TreeItemIndex[] => {
    if (!query.trim()) return items;

    // This is a simple implementation - in a real app you'd want more sophisticated filtering
    return items.filter(itemId => {
      const itemPath = itemId === 'root' ? '/' : itemId as string;
      const fileName = itemPath.split('/').pop() || '';
      return fileName.toLowerCase().includes(query.toLowerCase());
    });
  };

  return (
    <div className={`flex flex-col h-full bg-white ${className}`}>
      {/* Header with controls */}
      <div className="flex-shrink-0 border-b border-gray-200 bg-gray-50">
        <div className="flex items-center justify-between p-2">
          <h3 className="text-sm font-medium text-gray-700">Files</h3>
          <div className="flex items-center gap-1">
            <button
              onClick={() => setShowHiddenFiles(!showHiddenFiles)}
              className="p-1 hover:bg-gray-200 rounded transition-colors"
              title={showHiddenFiles ? "Hide hidden files" : "Show hidden files"}
            >
              {showHiddenFiles ? (
                <EyeOff className="w-4 h-4 text-gray-500" />
              ) : (
                <Eye className="w-4 h-4 text-gray-500" />
              )}
            </button>
            <button
              onClick={handleRefresh}
              className="p-1 hover:bg-gray-200 rounded transition-colors"
              title="Refresh file tree"
              disabled={isLoading}
            >
              <RotateCcw className={`w-4 h-4 text-gray-500 ${isLoading ? 'animate-spin' : ''}`} />
            </button>
          </div>
        </div>

        {/* Search box */}
        <div className="px-2 pb-2">
          <div className="relative">
            <Search className="absolute left-2 top-1/2 transform -translate-y-1/2 w-3 h-3 text-gray-400" />
            <input
              type="text"
              placeholder="Search files..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full pl-7 pr-2 py-1 text-xs border border-gray-300 rounded focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500"
            />
          </div>
        </div>
      </div>

      {/* Error display */}
      {error && (
        <div className="flex-shrink-0 bg-red-50 border-b border-red-200 px-2 py-1">
          <p className="text-xs text-red-600">{error}</p>
        </div>
      )}

      {/* Loading indicator */}
      {isLoading && (
        <div className="flex-shrink-0 bg-blue-50 border-b border-blue-200 px-2 py-1">
          <p className="text-xs text-blue-600">Loading...</p>
        </div>
      )}

      {/* File tree */}
      <div className="flex-1 overflow-hidden" ref={treeRef}>
        <UncontrolledTreeEnvironment
          key={refreshKey}
          dataProvider={dataProvider}
          getItemTitle={(item) => item.data.name}
          viewState={{
            ['vfs-tree']: {
              selectedItems,
              expandedItems,
              focusedItem
            }
          }}
          onSelectedItemsChange={(items, treeId) => {
            if (treeId === 'vfs-tree') {
              setSelectedItems(items);
            }
          }}
          onExpandedItemsChange={(items, treeId) => {
            if (treeId === 'vfs-tree') {
              setExpandedItems(items);
            }
          }}
          onFocusedItemChange={(item, treeId) => {
            if (treeId === 'vfs-tree') {
              setFocusedItem(item);
            }
          }}
          canDragAndDrop={true}
          canDropOnFolder={true}
          canReorderItems={true}
          canSearch={false} // We handle search ourselves
          canRename={true}
          renderItemTitle={renderFileTreeNode(
            handleDelete,
            handleCreateFile,
            handleCreateDirectory
          )}
          onPrimaryAction={handlePrimaryAction}
          onRenameItem={async (item, name) => {
            try {
              setIsLoading(true);
              setError(null);
              await dataProvider.onRenameItem(item, name);
            } catch (err) {
              const errorMessage = err instanceof Error ? err.message : 'Failed to rename item';
              setError(errorMessage);
              console.error('Rename error:', err);
            } finally {
              setIsLoading(false);
            }
          }}
          interactionMode={InteractionMode.ClickItemToExpand}
        >
          <div className="h-full overflow-auto">
            <Tree
              treeId="vfs-tree"
              rootItem="root"
              treeLabel="File System"
            />
          </div>
        </UncontrolledTreeEnvironment>
      </div>

      {/* Status bar */}
      <div className="flex-shrink-0 border-t border-gray-200 bg-gray-50 px-2 py-1">
        <p className="text-xs text-gray-500">
          {selectedItems.length > 0 && `${selectedItems.length} selected`}
          {selectedItems.length > 0 && expandedItems.length > 1 && ' • '}
          {expandedItems.length > 1 && `${expandedItems.length - 1} folders expanded`}
        </p>
      </div>
    </div>
  );
};
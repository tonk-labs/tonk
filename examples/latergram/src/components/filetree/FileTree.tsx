import React, { useState, useEffect, useRef } from 'react';
import {
  UncontrolledTreeEnvironment,
  Tree,
  TreeItemIndex,
  InteractionMode,
  TreeItem,
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
  className = '',
}) => {
  // Recreate data provider when rootPath changes
  const [dataProvider, setDataProvider] = useState(() => new VFSDataProvider(rootPath));

  // Update data provider when rootPath changes
  useEffect(() => {
    const newProvider = new VFSDataProvider(rootPath);

    // Clean up old provider
    if (dataProvider && dataProvider !== newProvider) {
      dataProvider.cleanup();
    }

    setDataProvider(newProvider);
    // Reset expansion to show the new root
    const base = ['root'];

    // If a specific root path is provided, expand to that path
    if (rootPath && rootPath !== '/') {
      const pathParts = rootPath.split('/').filter(p => p);
      let currentPath = '';

      // Build path incrementally and add each level
      pathParts.forEach(part => {
        currentPath = currentPath + '/' + part;
        base.push(currentPath);
      });

      // Also expand common subdirectories if in src
      if (rootPath.startsWith('/src')) {
        base.push('/src');
        if (rootPath === '/src/components') {
          base.push('/src/components');
        } else if (rootPath === '/src/views') {
          base.push('/src/views');
        } else if (rootPath === '/src/stores') {
          base.push('/src/stores');
        }
      }
    } else {
      // Default expansion for general browsing
      base.push('/src', '/src/components', '/src/views', '/src/stores');
    }

    setExpandedItems(base);
    setRefreshKey(prev => prev + 1);

    // Cleanup function
    return () => {
      newProvider.cleanup();
    };
  }, [rootPath]);
  const [searchQuery, setSearchQuery] = useState('');
  const [showHiddenFiles, setShowHiddenFiles] = useState(showHidden);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedItems, setSelectedItems] = useState<TreeItemIndex[]>([]);

  // Determine initial expanded items based on rootPath
  const getInitialExpandedItems = () => {
    const base = ['root'];

    // If a specific root path is provided, expand to that path
    if (rootPath && rootPath !== '/') {
      const pathParts = rootPath.split('/').filter(p => p);
      let currentPath = '';

      // Build path incrementally and add each level
      pathParts.forEach(part => {
        currentPath = currentPath + '/' + part;
        base.push(currentPath);
      });

      // Also expand common subdirectories if in src
      if (rootPath.startsWith('/src')) {
        base.push('/src');
        if (rootPath === '/src/components') {
          base.push('/src/components');
        } else if (rootPath === '/src/views') {
          base.push('/src/views');
        } else if (rootPath === '/src/stores') {
          base.push('/src/stores');
        }
      }
    } else {
      // Default expansion for general browsing
      base.push('/src', '/src/components', '/src/views', '/src/stores');
    }

    return base;
  };

  const [expandedItems, setExpandedItems] = useState<TreeItemIndex[]>(
    getInitialExpandedItems()
  );
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

  // Update search query in data provider when it changes
  useEffect(() => {
    dataProvider.setSearchQuery(searchQuery);
  }, [dataProvider, searchQuery]);

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
          const itemPath = id === 'root' ? '/' : (id as string);
          return itemPath !== path && !itemPath.startsWith(path + '/');
        })
      );

      setExpandedItems(prev =>
        prev.filter(id => {
          const itemPath = id === 'root' ? '/' : (id as string);
          return itemPath !== path && !itemPath.startsWith(path + '/');
        })
      );

      // Delete the item
      await dataProvider.deleteItem(path);
      onFileDelete?.(path);
    } catch (err) {
      const errorMessage =
        err instanceof Error ? err.message : 'Failed to delete file';
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
      const errorMessage =
        err instanceof Error ? err.message : 'Failed to create file';
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
      const errorMessage =
        err instanceof Error ? err.message : 'Failed to create directory';
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
    // Reset to initial expanded state based on rootPath
    setExpandedItems(getInitialExpandedItems());
    setSelectedItems([]);
    setFocusedItem(undefined);
  };

  const filterItems = (
    items: TreeItemIndex[],
    query: string
  ): TreeItemIndex[] => {
    if (!query.trim()) return items;

    // This is a simple implementation - in a real app you'd want more sophisticated filtering
    return items.filter(itemId => {
      const itemPath = itemId === 'root' ? '/' : (itemId as string);
      const fileName = itemPath.split('/').pop() || '';
      return fileName.toLowerCase().includes(query.toLowerCase());
    });
  };

  return (
    <div className={`flex flex-col h-full bg-white ${className}`}>
      {/* Header with controls */}
      <div className="flex-shrink-0 border-b border-gray-200 bg-gray-50">
        <div className="flex items-center justify-between p-2">
          <h3 className="text-sm font-medium text-gray-700">
            {rootPath && rootPath !== '/' ? `Files (${rootPath})` : 'Files'}
          </h3>
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => setShowHiddenFiles(!showHiddenFiles)}
              className="p-1 hover:bg-gray-200 rounded transition-colors"
              title={
                showHiddenFiles ? 'Hide hidden files' : 'Show hidden files'
              }
            >
              {showHiddenFiles ? (
                <EyeOff className="w-4 h-4 text-gray-500" />
              ) : (
                <Eye className="w-4 h-4 text-gray-500" />
              )}
            </button>
            <button
              type="button"
              onClick={handleRefresh}
              className="p-1 hover:bg-gray-200 rounded transition-colors"
              title="Refresh file tree"
              disabled={isLoading}
            >
              <RotateCcw
                className={`w-4 h-4 text-gray-500 ${isLoading ? 'animate-spin' : ''}`}
              />
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
              onChange={e => setSearchQuery(e.target.value)}
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
          getItemTitle={item => item.data.name}
          viewState={{
            ['vfs-tree']: {
              selectedItems,
              expandedItems,
              focusedItem,
            },
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
              const errorMessage =
                err instanceof Error ? err.message : 'Failed to rename item';
              setError(errorMessage);
              console.error('Rename error:', err);
            } finally {
              setIsLoading(false);
            }
          }}
          interactionMode={InteractionMode.ClickItemToExpand}
        >
          <div className="h-full overflow-auto">
            <Tree treeId="vfs-tree" rootItem="root" treeLabel="File System" />
          </div>
        </UncontrolledTreeEnvironment>
      </div>
    </div>
  );
};

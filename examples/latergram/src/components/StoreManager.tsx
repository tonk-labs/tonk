import React, { useState, useCallback, useEffect } from 'react';
import { Database, Layers } from 'lucide-react';
import { StoreBrowser } from './StoreBrowser';
import { StoreEditor } from './StoreEditor';
import { storeRegistry } from './StoreRegistry';
import { useVFSDiscovery } from './hooks/useVFSDiscovery';
import { compileTSCode, ensureTypeScriptLoaded } from './utils/tsCompiler';
import { extractStoreName } from './utils/nameExtractor';
import { getStoreTemplate } from './StoreTemplates';

export const StoreManager: React.FC = () => {
  const [selectedStoreId, setSelectedStoreId] = useState<string | null>(null);

  const { discoverFiles } = useVFSDiscovery({
    directory: '/src/stores',
    fileExtension: '.ts',
    extractName: extractStoreName,
    checkExisting: filePath => {
      return !!storeRegistry.getStoreByFilePath(filePath);
    },
    onFileFound: async (filePath, name, content) => {
      // Extract store ID from filename
      const storeId =
        filePath.split('/').pop()?.replace('.ts', '') || `store-${Date.now()}`;

      // Register placeholder store
      storeRegistry.register(storeId, () => ({}), {
        name: name,
        filePath: filePath,
        status: 'loading',
      });

      // Compile the store using module pattern, excluding self from context
      const result = await compileTSCode(content, {
        moduleType: 'CommonJS',
        outputFormat: 'module',
        excludeStoreId: storeId,
      });

      if (result.success && typeof result.output === 'function') {
        storeRegistry.update(storeId, result.output, 'success');
      } else {
        storeRegistry.update(storeId, null, 'error', result.error);
      }
    },
  });

  const handleStoreSelect = useCallback((storeId: string) => {
    setSelectedStoreId(storeId);
  }, []);

  const handleCreateStore = useCallback(
    async (templateName: string, storeName: string) => {
      try {
        // Create store in registry
        const storeId = storeRegistry.createStore(storeName);

        // Get template code
        const template = getStoreTemplate(templateName);
        if (!template) {
          throw new Error(`Template ${templateName} not found`);
        }

        // Customize template for this store
        let customizedCode = template.code;

        // Replace store name in the path
        const storeFileName =
          storeName.toLowerCase().replace(/store$/, '') + '-store';
        customizedCode = customizedCode.replace(
          /path: ['"][^'"]*['"]/,
          `path: '/src/stores/${storeFileName}.json'`
        );

        // Update file path in metadata
        const store = storeRegistry.getStore(storeId);
        if (store) {
          storeRegistry.updateMetadata(storeId, {
            filePath: `/src/stores/${storeId}.ts`,
          });

          // Create the file with template content
          const vfsService = (
            await import('../services/vfs-service')
          ).getVFSService();
          if (vfsService.isInitialized()) {
            try {
              await vfsService.writeFile(
                store.metadata.filePath,
                customizedCode,
                true
              );
              storeRegistry.updateMetadata(storeId, { status: 'success' });
            } catch (error) {
              console.error('Failed to create store file:', error);
              storeRegistry.updateMetadata(storeId, {
                status: 'error',
                error: 'Failed to create store file',
              });
            }
          }
        }

        // Select the new store
        setSelectedStoreId(storeId);
      } catch (error) {
        console.error('Failed to create store:', error);
        // Could add notification here
      }
    },
    []
  );

  const handleDeleteStore = useCallback(
    async (storeId: string) => {
      const store = storeRegistry.getStore(storeId);
      if (!store) return;

      try {
        // Delete from VFS
        const vfsService = (
          await import('../services/vfs-service')
        ).getVFSService();
        if (vfsService.isInitialized()) {
          try {
            await vfsService.deleteFile(store.metadata.filePath);
          } catch (error) {
            console.warn('Failed to delete store file:', error);
          }
        }

        // Remove from registry
        storeRegistry.deleteStore(storeId);

        // Clear selection if this store was selected
        if (selectedStoreId === storeId) {
          setSelectedStoreId(null);
        }
      } catch (error) {
        console.error('Failed to delete store:', error);
      }
    },
    [selectedStoreId]
  );

  return (
    <div className="h-full bg-gray-50 flex flex-col">
      {/* Header */}
      <div className="bg-white border-b border-gray-200 px-6 py-4 flex-shrink-0">
        <div className="flex items-center gap-3">
          <Database className="w-6 h-6 text-purple-500" />
          <div>
            <h1 className="text-xl font-semibold text-gray-900">
              Store Manager
            </h1>
            <p className="text-sm text-gray-600">
              Create and manage zustand stores for persistent, reactive state
            </p>
          </div>
        </div>
      </div>

      <div className="flex flex-1 overflow-hidden">
        {/* Left Panel - Store Browser */}
        <div className="w-80 border-r border-gray-200 bg-white flex-shrink-0">
          <StoreBrowser
            selectedStoreId={selectedStoreId}
            onStoreSelect={handleStoreSelect}
            onCreateStore={handleCreateStore}
            onDeleteStore={handleDeleteStore}
          />
        </div>

        {/* Main Content */}
        <div className="flex-1 flex flex-col overflow-hidden">
          {selectedStoreId ? (
            /* Store Editor */
            <div className="flex-1 p-6 overflow-hidden">
              <StoreEditor storeId={selectedStoreId} height="100%" />
            </div>
          ) : (
            /* Welcome Screen */
            <div className="flex-1 flex items-center justify-center">
              <div className="text-center max-w-md">
                <div className="flex justify-center mb-6">
                  <div className="relative">
                    <Database className="w-16 h-16 text-gray-300" />
                    <Layers className="w-8 h-8 text-purple-500 absolute -bottom-1 -right-1" />
                  </div>
                </div>

                <h2 className="text-xl font-semibold text-gray-900 mb-3">
                  Welcome to Store Manager
                </h2>

                <p className="text-gray-600 mb-6">
                  Create domain-specific zustand stores that persist across
                  component re-renders, hot reloads, and page refreshes. All
                  stores are automatically available as hooks in your
                  components.
                </p>

                <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 text-left">
                  <h3 className="font-medium text-blue-900 mb-2">
                    Getting Started:
                  </h3>
                  <ul className="text-sm text-blue-800 space-y-1">
                    <li>• Click "New Store" to create your first store</li>
                    <li>
                      • Choose from pre-built templates or start from scratch
                    </li>
                    <li>
                      • Stores are automatically compiled and available in
                      components
                    </li>
                    <li>
                      • State persists across hot reloads and page refreshes
                    </li>
                  </ul>
                </div>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

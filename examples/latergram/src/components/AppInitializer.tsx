import { useEffect, useState, useRef } from 'react';
import { getVFSService } from '../services/vfs-service';
import { componentRegistry } from './ComponentRegistry';
import { storeRegistry } from './StoreRegistry';
import { ensureTypeScriptLoaded, compileTSCode } from './utils/tsCompiler';
import { extractComponentName, extractStoreName } from './utils/nameExtractor';
import { useComponentWatcher } from './hooks/useComponentWatcher';

interface AppInitializerProps {
  children: React.ReactNode;
}

/**
 * AppInitializer ensures all stores and components are compiled on app startup
 */
export const AppInitializer: React.FC<AppInitializerProps> = ({ children }) => {
  const [isInitialized, setIsInitialized] = useState(false);
  const [loadingMessage, setLoadingMessage] = useState('Initializing...');
  const initStartedRef = useRef(false);
  const { watchComponent } = useComponentWatcher();
  const vfs = getVFSService();

  useEffect(() => {
    if (initStartedRef.current) return;
    initStartedRef.current = true;

    const initializeApp = async () => {
      try {
        await ensureTypeScriptLoaded();

        // VFS should already be initialized before React renders
        if (!vfs.isInitialized()) {
          console.warn('VFS not initialized, skipping file discovery');
          setIsInitialized(true);
          return;
        }

        // Load all stores first
        setLoadingMessage('Loading stores...');
        await loadAllStores();

        // Then load all components
        setLoadingMessage('Loading components...');
        await loadAllComponents();

        setIsInitialized(true);
      } catch (error) {
        console.error('Failed to initialize app:', error);
        setIsInitialized(true);
      }
    };

    initializeApp();
  }, []);

  const loadAllStores = async () => {
    try {
      const storeFiles = await vfs.listDirectory('/src/stores');

      for (const fileInfo of storeFiles as any[]) {
        const fileName =
          typeof fileInfo === 'string'
            ? fileInfo
            : fileInfo.name || fileInfo.path;

        if (!fileName || !fileName.endsWith('.ts')) {
          continue;
        }

        const filePath = `/src/stores/${fileName}`;

        // Check if store already exists
        if (storeRegistry.getStoreByFilePath(filePath)) {
          continue;
        }

        try {
          const content = await vfs.readBytesAsString(filePath);
          const storeName = extractStoreName(content, fileName);
          const storeId = fileName.replace('.ts', '') || `store-${Date.now()}`;

          // Register placeholder store
          storeRegistry.register(storeId, () => ({}), {
            name: storeName,
            filePath: filePath,
            status: 'loading',
          });

          // Compile the store
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
        } catch (error) {
          console.warn(`Failed to load store ${filePath}:`, error);
        }
      }
    } catch (error) {
      console.warn('Failed to discover stores:', error);
    }
  };

  const loadAllComponents = async () => {
    try {
      const componentFiles = await vfs.listDirectory('/src/components');

      for (const fileInfo of componentFiles as any[]) {
        const fileName =
          typeof fileInfo === 'string'
            ? fileInfo
            : fileInfo.name || fileInfo.path;

        if (!fileName || !fileName.endsWith('.tsx')) {
          continue;
        }

        const filePath = `/src/components/${fileName}`;

        // Check if component already exists
        if (componentRegistry.getComponentByFilePath(filePath)) {
          continue;
        }

        try {
          const content = await vfs.readBytesAsString(filePath);
          const componentName = extractComponentName(content, fileName);

          // Create component in registry
          const componentId = componentRegistry.createComponent(
            componentName,
            filePath
          );

          // Set up file watcher for the component
          await watchComponent(componentId, filePath);
        } catch (error) {
          console.warn(`Failed to load component ${filePath}:`, error);
        }
      }
    } catch (error) {
      console.warn('Failed to discover components:', error);
    }
  };

  if (!isInitialized) {
    return (
      <div className="h-screen flex items-center justify-center bg-gray-50">
        <div className="text-center">
          <div className="inline-block h-8 w-8 animate-spin rounded-full border-4 border-solid border-blue-500 border-r-transparent"></div>
          <p className="mt-4 text-sm text-gray-600">{loadingMessage}</p>
        </div>
      </div>
    );
  }

  return <>{children}</>;
};

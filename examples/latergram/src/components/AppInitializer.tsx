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

        // Load all stores first and wait for them to complete
        setLoadingMessage('Loading stores...');
        await loadAllStores();

        // Small delay to ensure store proxies are properly updated
        await new Promise(resolve => setTimeout(resolve, 100));

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
      const storePromises: Promise<void>[] = [];

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

        // Create a promise for each store compilation
        const storePromise = (async () => {
          try {
            const content = await vfs.readFile(filePath);
            const storeName = extractStoreName(content, fileName);
            const storeId = fileName.replace('.ts', '') || `store-${Date.now()}`;

            // Register placeholder store immediately so it's available in the registry
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
        })();

        storePromises.push(storePromise);
      }

      // Wait for all stores to finish loading
      await Promise.all(storePromises);
    } catch (error) {
      console.warn('Failed to discover stores:', error);
    }
  };

  const loadAllComponents = async () => {
    try {
      const componentFiles = await vfs.listDirectory('/src/components');
      const componentPaths: { id: string; filePath: string; name: string }[] = [];

      // First pass: Register all components so they're available in the registry
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
          const content = await vfs.readFile(filePath);
          const componentName = extractComponentName(content, fileName);

          // Create component in registry with loading status
          const componentId = componentRegistry.createComponent(
            componentName,
            filePath
          );

          componentPaths.push({ id: componentId, filePath, name: componentName });
        } catch (error) {
          console.warn(`Failed to register component ${filePath}:`, error);
        }
      }

      // Second pass: Compile all components with all other components available as placeholders
      const compilationPromises = componentPaths.map(async ({ id, filePath }) => {
        try {
          await watchComponent(id, filePath);
        } catch (error) {
          console.warn(`Failed to compile component ${filePath}:`, error);
        }
      });

      await Promise.all(compilationPromises);

      // Third pass: Recompile any components that had errors due to missing dependencies
      const failedComponents = componentRegistry.getComponentsByStatus('error');
      if (failedComponents.length > 0) {
        console.log(`Retrying ${failedComponents.length} failed components...`);

        // Wait a bit for all components to be registered
        await new Promise(resolve => setTimeout(resolve, 200));

        const retryPromises = failedComponents.map(async comp => {
          try {
            const content = await vfs.readFile(comp.metadata.filePath);
            await watchComponent(comp.id, comp.metadata.filePath);
          } catch (error) {
            console.warn(`Retry failed for component ${comp.metadata.filePath}:`, error);
          }
        });

        await Promise.all(retryPromises);
      }
    } catch (error) {
      console.warn('Failed to discover components:', error);
    }
  };

  if (!isInitialized) {
    return (
      <div className="h-screen flex items-center justify-center bg-gray-50">
        <div className="text-center">
          <div className="inline-block animate-pulse"><div className="h-8 w-8 animate-spin rounded-full border-4 border-solid border-gray-500 border-r-transparent"/></div>
        </div>
      </div>
    );
  }

  return <>{children}</>;
};

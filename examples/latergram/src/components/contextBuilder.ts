import { componentRegistry } from './ComponentRegistry';
import { storeRegistry } from './StoreRegistry';

export const sanitizeComponentName = (name: string): string => {
  return (
    name.replace(/[^a-zA-Z0-9]/g, '').replace(/^[0-9]/, 'Component$&') ||
    'UnnamedComponent'
  );
};

export const sanitizeStoreName = (name: string): string => {
  return (
    name.replace(/[^a-zA-Z0-9]/g, '').replace(/^[0-9]/, 'Store$&') ||
    'UnnamedStore'
  );
};

export const buildAvailablePackages = (excludeStoreId?: string) => {
  const basePackages = {
    React: (window as any).React,
    useState: (window as any).React?.useState,
    useEffect: (window as any).React?.useEffect,
    useCallback: (window as any).React?.useCallback,
    useMemo: (window as any).React?.useMemo,
    useRef: (window as any).React?.useRef,
    useReducer: (window as any).React?.useReducer,
    useContext: (window as any).React?.useContext,
    Fragment: (window as any).React?.Fragment,
    // Add zustand imports for store creation
    create: (window as any).zustand?.create,
    sync: (window as any).sync, // Our custom sync middleware
  };

  // Add all registered components
  const componentPackages: { [key: string]: any } = {};
  componentRegistry.getAllComponents().forEach(comp => {
    if (comp.metadata.status === 'success') {
      const sanitizedName = sanitizeComponentName(comp.metadata.name);
      if (sanitizedName !== 'UnnamedComponent') {
        componentPackages[sanitizedName] = comp.proxy;
      }
    }
  });

  // Add all registered stores, excluding the one being compiled
  const storePackages: { [key: string]: any } = {};
  storeRegistry.getAllStores().forEach(store => {
    if (store.metadata.status === 'success' && store.id !== excludeStoreId) {
      // Use the store name as-is from the code (extracted by AST)
      // Only sanitize to remove invalid characters, don't add prefixes/suffixes
      const storeName = store.metadata.name;
      const sanitizedName =
        storeName.replace(/[^a-zA-Z0-9]/g, '') || 'UnnamedStore';

      if (sanitizedName !== 'UnnamedStore') {
        storePackages[sanitizedName] = store.proxy;
      }
    }
  });

  return { ...basePackages, ...componentPackages, ...storePackages };
};

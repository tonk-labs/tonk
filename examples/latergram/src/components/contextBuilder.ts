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

// Create a safe store proxy that returns undefined instead of throwing
const createSafeStoreProxy = (storeName: string) => {
  return new Proxy(() => undefined, {
    get: (target, prop) => {
      console.warn(`Store '${storeName}' is still loading. Property '${String(prop)}' accessed.`);
      return undefined;
    },
    apply: () => {
      console.warn(`Store '${storeName}' is still loading. Returning undefined.`);
      return undefined;
    }
  });
};

// Create a safe component proxy that returns a placeholder component
const createSafeComponentProxy = (componentName: string, componentId: string) => {
  // This creates a dynamic proxy that checks if the real component is available
  const DynamicPlaceholder = () => {
    const React = (window as any).React;
    const [, forceUpdate] = React.useReducer((x: number) => x + 1, 0);

    React.useEffect(() => {
      // Check if the real component is now available
      const checkComponent = () => {
        const realComponent = componentRegistry.getComponent(componentId);
        if (realComponent && realComponent.metadata.status === 'success') {
          forceUpdate();
        }
      };

      // Set up a listener for component updates
      const unsubscribe = componentRegistry.onUpdate(componentId, () => {
        checkComponent();
      });

      // Also listen to context updates (when new components are registered)
      const contextUnsub = componentRegistry.onContextUpdate(() => {
        checkComponent();
      });

      // Check immediately and periodically
      checkComponent();
      const interval = setInterval(checkComponent, 500);

      return () => {
        unsubscribe();
        contextUnsub();
        clearInterval(interval);
      };
    }, []);

    // Try to get the real component
    const realComponent = componentRegistry.getComponent(componentId);
    if (realComponent && realComponent.metadata.status === 'success') {
      return React.createElement(realComponent.proxy);
    }

    // Show placeholder
    return React.createElement(
      'div',
      {
        style: {
          margin: '8px',
          padding: '8px',
          background: '#f0f0f0',
          border: '1px dashed #999',
          borderRadius: '4px',
          fontSize: '12px',
          color: '#666'
        }
      },
      `Loading ${componentName}...`
    );
  };
  DynamicPlaceholder.displayName = `${componentName}(Loading)`;
  return DynamicPlaceholder;
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

  // Add all registered components, including loading ones with safe proxies
  const componentPackages: { [key: string]: any } = {};
  componentRegistry.getAllComponents().forEach(comp => {
    const sanitizedName = sanitizeComponentName(comp.metadata.name);
    if (sanitizedName !== 'UnnamedComponent') {
      // If component is successfully loaded, use the real proxy
      if (comp.metadata.status === 'success') {
        componentPackages[sanitizedName] = comp.proxy;
      } else {
        // If component is still loading or has errors, provide a safe placeholder
        componentPackages[sanitizedName] = createSafeComponentProxy(sanitizedName, comp.id);
      }
    }
  });

  // Add all registered stores, including loading ones with safe proxies
  const storePackages: { [key: string]: any } = {};
  storeRegistry.getAllStores().forEach(store => {
    if (store.id === excludeStoreId) {
      return; // Skip the store being compiled
    }

    const storeName = store.metadata.name;
    const sanitizedName =
      storeName.replace(/[^a-zA-Z0-9]/g, '') || 'UnnamedStore';

    if (sanitizedName !== 'UnnamedStore') {
      // If store is successfully loaded, use the real proxy
      if (store.metadata.status === 'success') {
        storePackages[sanitizedName] = store.proxy;
      } else {
        // If store is still loading or has errors, provide a safe proxy
        storePackages[sanitizedName] = createSafeStoreProxy(sanitizedName);
      }
    }
  });

  return { ...basePackages, ...componentPackages, ...storePackages };
};

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
      console.warn(
        `Store '${storeName}' is still loading. Property '${String(prop)}' accessed.`
      );
      return undefined;
    },
    apply: () => {
      console.warn(
        `Store '${storeName}' is still loading. Returning undefined.`
      );
      return undefined;
    },
  });
};

// Create a safe component proxy that returns a placeholder component
const createSafeComponentProxy = (
  componentName: string,
  componentId: string
) => {
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
          color: '#666',
        },
      },
      `Loading ${componentName}...`
    );
  };
  DynamicPlaceholder.displayName = `${componentName}(Loading)`;
  return DynamicPlaceholder;
};

export interface RouterContext {
  navigate: (
    to: string | number,
    options?: { replace?: boolean; state?: any }
  ) => void;
  location: {
    pathname: string;
    search: string;
    hash: string;
    state: any;
    key: string;
  };
  params: Record<string, string | undefined>;
}

export const buildAvailablePackages = (
  excludeId?: string,
  routerContext?: RouterContext
) => {
  // Create bridged router functions/components if context is provided
  const createBridgedLink = () => {
    if (!routerContext) {
      return (window as any).ReactRouterDOM?.Link;
    }

    const React = (window as any).React;
    return (props: any) => {
      const { to, replace, state, onClick, children, ...rest } = props;

      const handleClick = (e: any) => {
        // Call custom onClick first if it exists
        if (onClick) {
          onClick(e);
        }

        // Only navigate if the custom onClick didn't prevent default
        if (!e.defaultPrevented) {
          e.preventDefault(); // Prevent the default <a> behavior
          routerContext.navigate(to, { replace, state });
        }
      };

      return React.createElement(
        'a',
        {
          ...rest,
          href: to,
          onClick: handleClick,
        },
        children
      );
    };
  };

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
    // Add React Router imports - bridged if context provided
    Link: createBridgedLink(),
    NavLink: routerContext
      ? createBridgedLink()
      : (window as any).ReactRouterDOM?.NavLink,
    useNavigate: routerContext
      ? () => routerContext.navigate
      : (window as any).ReactRouterDOM?.useNavigate,
    useLocation: routerContext
      ? () => routerContext.location
      : (window as any).ReactRouterDOM?.useLocation,
    useParams: routerContext
      ? () => routerContext.params
      : (window as any).ReactRouterDOM?.useParams,
    useSearchParams: (window as any).ReactRouterDOM?.useSearchParams,
  };

  // Add all registered components, including loading ones with safe proxies
  const componentPackages: { [key: string]: any } = {};
  componentRegistry.getAllComponents().forEach(comp => {
    // Skip if this is the component being compiled
    if (comp.id === excludeId) {
      return;
    }

    const sanitizedName = sanitizeComponentName(comp.metadata.name);
    if (sanitizedName !== 'UnnamedComponent') {
      // If component is successfully loaded, use the real proxy
      if (comp.metadata.status === 'success') {
        componentPackages[sanitizedName] = comp.proxy;
      } else {
        // If component is still loading or has errors, provide a safe placeholder
        componentPackages[sanitizedName] = createSafeComponentProxy(
          sanitizedName,
          comp.id
        );
      }
    }
  });

  // Add all registered stores, including loading ones with safe proxies
  const storePackages: { [key: string]: any } = {};
  storeRegistry.getAllStores().forEach(store => {
    // Skip if this is the store being compiled
    if (store.id === excludeId) {
      return;
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

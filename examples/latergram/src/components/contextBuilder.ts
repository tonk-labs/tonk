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

const createSafeComponentProxy = (
  componentName: string,
  componentId: string
) => {
  const DynamicPlaceholder = () => {
    const React = (window as any).React;
    const [, forceUpdate] = React.useReducer((x: number) => x + 1, 0);

    React.useEffect(() => {
      const checkComponent = () => {
        const realComponent = componentRegistry.getComponent(componentId);
        if (realComponent && realComponent.metadata.status === 'success') {
          forceUpdate();
        }
      };

      const unsubscribe = componentRegistry.onUpdate(componentId, () => {
        checkComponent();
      });

      const contextUnsub = componentRegistry.onContextUpdate(() => {
        checkComponent();
      });

      checkComponent();
      const interval = setInterval(checkComponent, 500);

      return () => {
        unsubscribe();
        contextUnsub();
        clearInterval(interval);
      };
    }, []);

    const realComponent = componentRegistry.getComponent(componentId);
    if (realComponent && realComponent.metadata.status === 'success') {
      return React.createElement(realComponent.proxy);
    }

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
  const createBridgedLink = () => {
    if (!routerContext) {
      return (window as any).ReactRouterDOM?.Link;
    }

    const React = (window as any).React;
    return (props: any) => {
      const { to, replace, state, onClick, children, ...rest } = props;

      const handleClick = (e: any) => {
        if (onClick) {
          onClick(e);
        }

        if (!e.defaultPrevented) {
          e.preventDefault();
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

  const ChakraUI = (window as any).ChakraUI || {};

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
    create: (window as any).zustand?.create,
    sync: (window as any).sync,
    createToaster: (window as any).createToaster,
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
    ...ChakraUI,
  };

  const componentPackages: { [key: string]: any } = {};
  componentRegistry.getAllComponents().forEach(comp => {
    if (comp.id === excludeId) {
      return;
    }

    const sanitizedName = sanitizeComponentName(comp.metadata.name);
    if (sanitizedName !== 'UnnamedComponent') {
      if (comp.metadata.status === 'success') {
        componentPackages[sanitizedName] = comp.proxy;
      } else {
        componentPackages[sanitizedName] = createSafeComponentProxy(
          sanitizedName,
          comp.id
        );
      }
    }
  });

  const storePackages: { [key: string]: any } = {};
  storeRegistry.getAllStores().forEach(store => {
    if (store.id === excludeId) {
      return;
    }

    const storeName = store.metadata.name;
    const sanitizedName =
      storeName.replace(/[^a-zA-Z0-9]/g, '') || 'UnnamedStore';

    if (sanitizedName !== 'UnnamedStore') {
      if (store.metadata.status === 'success') {
        storePackages[sanitizedName] = store.proxy;
      } else {
        storePackages[sanitizedName] = createSafeStoreProxy(sanitizedName);
      }
    }
  });

  return { ...basePackages, ...componentPackages, ...storePackages };
};

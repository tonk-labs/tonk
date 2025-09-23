import { componentRegistry } from './ComponentRegistry';

export const sanitizeComponentName = (name: string): string => {
  return (
    name.replace(/[^a-zA-Z0-9]/g, '').replace(/^[0-9]/, 'Component$&') ||
    'UnnamedComponent'
  );
};

export const buildAvailablePackages = () => {
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

  return { ...basePackages, ...componentPackages };
};

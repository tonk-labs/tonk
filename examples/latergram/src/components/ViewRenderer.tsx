import type { DocumentData } from '@tonk/core';
import { AlertCircle, Edit3, FileX, Loader2 } from 'lucide-react';
import type React from 'react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Link, useLocation, useNavigate, useParams } from 'react-router-dom';
import { cn } from '../lib/utils';
import { getVFSService } from '../services/vfs-service';
import { bytesToString } from '../utils/vfs-utils';
import { buildAvailablePackages } from './contextBuilder';
import { createInlineErrorBoundary } from './errors/createInlineErrorBoundary';

interface ViewRendererProps {
  viewPath: string;
  className?: string;
}

export const ViewRenderer: React.FC<ViewRendererProps> = ({
  viewPath,
  className = '',
}) => {
  const sandboxRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<any>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [viewExists, setViewExists] = useState(false);
  const [renderError, setRenderError] = useState<string | null>(null);
  const [compiledComponent, setCompiledComponent] =
    useState<React.ComponentType | null>(null);

  const vfs = getVFSService();

  // Extract router context to bridge into the sandbox
  const navigate = useNavigate();
  const location = useLocation();
  const params = useParams();

  const compileView = useCallback(
    async (code: string, retryCount = 0): Promise<any> => {
      try {
        const ts = (window as any).ts;
        if (!ts) {
          throw new Error('TypeScript compiler not loaded');
        }

        const compiled = ts.transpileModule(code, {
          compilerOptions: {
            jsx: ts.JsxEmit.React,
            module: ts.ModuleKind.CommonJS,
            target: ts.ScriptTarget.ES2020,
          },
        });

        // Get fresh context with bridged router context
        const routerContext = {
          navigate,
          location,
          params,
        };

        const freshPackages = buildAvailablePackages(undefined, routerContext);
        const contextKeys = Object.keys(freshPackages);
        const contextValues = Object.values(freshPackages);

        const moduleFactory = new Function(
          ...contextKeys,
          `
        const exports = {};
        const module = { exports };

        ${compiled.outputText}

        return module.exports.default || module.exports;
      `
        );

        const component = moduleFactory(...contextValues);
        return component;
      } catch (error) {
        const errorMessage =
          error instanceof Error ? error.message : 'Compilation failed';
        error instanceof Error ? error.message : 'Compilation failed';

        // If it's a store-related error and we haven't retried too many times, try again
        if (retryCount < 3 && errorMessage.includes('is not defined')) {
          console.warn(
            `View compilation failed, retrying (attempt ${retryCount + 1}):`,
            errorMessage
          );
          await new Promise(resolve =>
            setTimeout(resolve, 200 * (retryCount + 1))
          );
          return compileView(code, retryCount + 1);
        }

        console.error('View compilation failed:', errorMessage);
        throw new Error(errorMessage);
      }
    },
    [navigate, location, params]
  );

  const loadView = useCallback(async () => {
    setIsLoading(true);
    setRenderError(null);

    try {
      // Check if view file exists
      const exists = await vfs.exists(viewPath);
      setViewExists(exists);

      if (!exists) {
        setIsLoading(false);
        return;
      }

      // Read and compile the view
      const sourceCode = await vfs.readBytesAsString(viewPath);
      const component = await compileView(sourceCode);
      setCompiledComponent(() => component);
    } catch (error) {
      console.error('Failed to load view:', error);
      setRenderError(
        error instanceof Error ? error.message : 'Failed to load view'
      );
    } finally {
      setIsLoading(false);
    }
  }, [viewPath, vfs, compileView]);

  // Load the view when component mounts or path changes
  useEffect(() => {
    if (vfs.isInitialized()) {
      loadView();
    }
  }, [loadView, vfs, viewPath]);

  // Watch for file changes
  useEffect(() => {
    if (!vfs.isInitialized() || !viewExists) return;

    const watchFile = async () => {
      try {
        const watchId = await vfs.watchFile(
          viewPath,
          async (content: DocumentData) => {
            try {
              const sourceCode = bytesToString(content);
              const component = await compileView(sourceCode);
              setCompiledComponent(() => component);
              setRenderError(null);
            } catch (error) {
              setRenderError(
                error instanceof Error ? error.message : 'Compilation failed'
              );
            }
          }
        );

        return () => {
          vfs.unwatchFile(watchId).catch(console.error);
        };
      } catch (error) {
        console.error('Failed to watch view file:', error);
      }
    };

    let cleanup: (() => void) | undefined;
    watchFile().then(fn => {
      cleanup = fn;
    });

    return () => {
      if (cleanup) cleanup();
    };
  }, [viewPath, vfs, viewExists, compileView]);

  // Render the component
  useEffect(() => {
    if (!compiledComponent || !sandboxRef.current) {
      if (rootRef.current && sandboxRef.current) {
        try {
          if ((window as any).ReactDOM?.unmountComponentAtNode) {
            (window as any).ReactDOM.unmountComponentAtNode(sandboxRef.current);
          } else if (rootRef.current.unmount) {
            rootRef.current.unmount();
          }
        } catch (error) {
          console.warn('Error unmounting component:', error);
        }
        rootRef.current = null;
      }
      return;
    }

    const renderComponent = () => {
      const React = (window as any).React;
      const ReactDOM = (window as any).ReactDOM;

      if (!React || !ReactDOM) {
        setRenderError('React or ReactDOM not available');
        return;
      }

      try {
        setRenderError(null);

        // Use the shared error boundary creator for page/view errors
        const viewName =
          viewPath.split('/').pop()?.replace('.tsx', '') || 'View';
        const PageErrorBoundary = createInlineErrorBoundary(
          React,
          viewName,
          viewPath
        );

        // Wrap the component with the error boundary
        const element = React.createElement(
          PageErrorBoundary,
          {},
          React.createElement(compiledComponent)
        );

        if (!rootRef.current && sandboxRef.current) {
          if (ReactDOM.createRoot) {
            rootRef.current = ReactDOM.createRoot(sandboxRef.current);
          } else {
            rootRef.current = {
              render: (el: any) => ReactDOM.render(el, sandboxRef.current),
              unmount: () =>
                ReactDOM.unmountComponentAtNode(sandboxRef.current),
            };
          }
        }

        if (rootRef.current) {
          rootRef.current.render(element);
        }
      } catch (error) {
        const errorMessage =
          error instanceof Error ? error.message : 'Render failed';
        setRenderError(errorMessage);
        console.error('View render error:', error);
      }
    };

    const timeoutId = setTimeout(renderComponent, 100);

    return () => {
      clearTimeout(timeoutId);
    };
  }, [compiledComponent, viewPath]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (rootRef.current && sandboxRef.current) {
        try {
          if ((window as any).ReactDOM?.unmountComponentAtNode) {
            (window as any).ReactDOM.unmountComponentAtNode(sandboxRef.current);
          } else if (rootRef.current.unmount) {
            rootRef.current.unmount();
          }
        } catch (error) {
          console.warn('Error during cleanup:', error);
        }
      }
    };
  }, []);

  // Show loading state
  if (isLoading) {
    return (
      <div
        className={`flex items-center justify-center h-screen bg-gray-50 ${className}`}
      >
        <div className="text-center">
          <Loader2 className="w-8 h-8 animate-spin text-blue-500 mx-auto mb-4" />
          <p className="text-gray-600">Loading view...</p>
        </div>
      </div>
    );
  }

  // Show skeleton if view doesn't exist
  if (!viewExists) {
    return (
      <div
        className={`flex items-center justify-center h-screen bg-gray-50 ${className}`}
      >
        <div className="text-center max-w-md mx-auto p-8">
          <FileX className="w-16 h-16 text-gray-400 mx-auto mb-4" />
          <h2 className="text-xl font-semibold text-gray-800 mb-2">
            No View Found
          </h2>
          <p className="text-gray-600 mb-6">
            The view at{' '}
            <code className="bg-gray-100 px-2 py-1 rounded text-sm">
              {viewPath}
            </code>{' '}
            doesn't exist yet.
          </p>
          <Link
            to={`/editor/pages?file=${encodeURIComponent(viewPath)}`}
            className="inline-flex items-center gap-2 px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
          >
            <Edit3 className="w-4 h-4" />
            Open Editor to Create View
          </Link>
        </div>
      </div>
    );
  }

  // Show error state
  if (renderError) {
    return (
      <div className={`h-screen bg-gray-50 ${className}`}>
        <div className="flex items-center justify-center h-full">
          <div className="max-w-2xl mx-auto p-8">
            <div className="bg-red-50 border border-red-200 rounded-lg p-6">
              <div className="flex items-start gap-3">
                <AlertCircle className="w-6 h-6 text-red-600 mt-0.5 flex-shrink-0" />
                <div className="flex-1">
                  <h3 className="font-semibold text-red-800 mb-2">
                    View Error
                  </h3>
                  <div className="text-sm text-red-700 font-mono whitespace-pre-wrap">
                    {renderError}
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    );
  }

  // Render the view
  return (
    <div className={cn(className, 'relative')}>
      <div ref={sandboxRef} className="min-h-screen -z-10" />
    </div>
  );
};

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
import { errorStyles } from './errors/errorBoundaryStyles';
import { sendErrorToAgent } from './errors/sendErrorToAgent';

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
  const [errorSent, setErrorSent] = useState(false);

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
    setErrorSent(false);

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
    const handleSendToAgent = () => {
      setErrorSent(true);
      const error = new Error(renderError);
      const errorInfo = { componentStack: '' };
      const viewName = viewPath.split('/').pop()?.replace('.tsx', '') || 'View';
      sendErrorToAgent(error, errorInfo, viewName, viewPath);
    };

    const styles = errorStyles.page;
    const viewName = viewPath.split('/').pop()?.replace('.tsx', '') || 'View';

    return (
      <div className={`${styles.container} ${className}`}>
        <div className={styles.wrapper}>
          <div className={styles.box}>
            <div className={styles.content}>
              <div className={styles.iconWrapper}>
                <div className={styles.icon}>
                  <span className={styles.iconText}>!</span>
                </div>
              </div>
              <div className={styles.body}>
                <h2 className={styles.title}>Page Error: {viewName}</h2>
                <p className={styles.path}>{viewPath}</p>
                <div className={styles.errorBox}>
                  <p className={styles.errorMessage}>{renderError}</p>
                </div>
                <button
                  type="button"
                  onClick={handleSendToAgent}
                  disabled={errorSent}
                  className={styles.sendButton}
                >
                  {errorSent
                    ? '✓ Sent to AI Agent'
                    : 'Ask AI to Fix This Error'}
                </button>
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

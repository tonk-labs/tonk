import React, { useRef, useEffect, useState, useCallback } from 'react';
import { AlertCircle, FileX, Loader2, Edit3 } from 'lucide-react';
import { getVFSService } from '../services/vfs-service';
import { buildAvailablePackages } from './contextBuilder';
import { Link } from 'react-router-dom';

interface ViewRendererProps {
  viewPath: string;
  className?: string;
  showEditor?: boolean;
}

export const ViewRenderer: React.FC<ViewRendererProps> = ({
  viewPath,
  className = '',
  showEditor = false,
}) => {
  const sandboxRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<any>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [viewExists, setViewExists] = useState(false);
  const [renderError, setRenderError] = useState<string | null>(null);
  const [compiledComponent, setCompiledComponent] = useState<React.ComponentType | null>(null);

  const vfs = getVFSService();

  const compileView = useCallback(async (code: string) => {
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

      // Get fresh context
      const freshPackages = buildAvailablePackages();
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
      const errorMessage = error instanceof Error ? error.message : 'Compilation failed';
      console.error('View compilation failed:', errorMessage);
      throw new Error(errorMessage);
    }
  }, []);

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
      const sourceCode = await vfs.readFile(viewPath);
      const component = await compileView(sourceCode);
      setCompiledComponent(() => component);

    } catch (error) {
      console.error('Failed to load view:', error);
      setRenderError(error instanceof Error ? error.message : 'Failed to load view');
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
        const watchId = await vfs.watchFile(viewPath, async (content: string) => {
          try {
            const component = await compileView(content);
            setCompiledComponent(() => component);
            setRenderError(null);
          } catch (error) {
            setRenderError(error instanceof Error ? error.message : 'Compilation failed');
          }
        });

        return () => {
          vfs.unwatchFile(watchId).catch(console.error);
        };
      } catch (error) {
        console.error('Failed to watch view file:', error);
      }
    };

    let cleanup: (() => void) | undefined;
    watchFile().then(fn => { cleanup = fn; });

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

        // Create an error boundary for the page/view
        class PageErrorBoundary extends React.Component {
          constructor(props: any) {
            super(props);
            this.state = { hasError: false, error: null };
          }

          static getDerivedStateFromError(error: any) {
            return { hasError: true, error };
          }

          componentDidCatch(error: any, errorInfo: any) {
            console.error(`[View: ${viewPath}] Runtime error:`, error, errorInfo);
          }

          render() {
            if ((this.state as any).hasError) {
              const error = (this.state as any).error;
              const viewName = viewPath.split('/').pop()?.replace('.tsx', '') || 'View';

              return React.createElement(
                'div',
                { className: 'flex items-center justify-center min-h-screen bg-gray-50 p-8' },
                React.createElement(
                  'div',
                  { className: 'w-full max-w-2xl' },
                  React.createElement(
                    'div',
                    { className: 'bg-red-50 border-2 border-red-500 rounded-lg p-6' },
                    React.createElement(
                      'div',
                      { className: 'flex items-start gap-4' },
                      [
                        React.createElement(
                          'div',
                          { key: 'icon', className: 'flex-shrink-0' },
                          React.createElement(
                            'div',
                            { className: 'w-10 h-10 bg-red-500 rounded flex items-center justify-center' },
                            React.createElement(
                              'span',
                              { className: 'text-white font-bold text-xl' },
                              '!'
                            )
                          )
                        ),
                        React.createElement(
                          'div',
                          { key: 'content', className: 'flex-1' },
                          [
                            React.createElement(
                              'h2',
                              { key: 'title', className: 'text-red-800 font-bold text-lg mb-2' },
                              `Page Error: ${viewName}`
                            ),
                            React.createElement(
                              'p',
                              { key: 'path', className: 'text-red-700 text-sm font-mono mb-3' },
                              viewPath
                            ),
                            React.createElement(
                              'div',
                              { key: 'error', className: 'bg-red-100 rounded p-3 mb-3' },
                              React.createElement(
                                'p',
                                { className: 'text-red-800 font-mono text-sm' },
                                error?.message || 'Unknown error occurred'
                              )
                            ),
                            React.createElement(
                              'details',
                              { key: 'stack', className: 'text-sm' },
                              [
                                React.createElement(
                                  'summary',
                                  { key: 'summary', className: 'text-red-600 cursor-pointer hover:text-red-800 font-medium' },
                                  'Show stack trace'
                                ),
                                React.createElement(
                                  'pre',
                                  {
                                    key: 'trace',
                                    className: 'mt-3 p-3 bg-white border border-red-200 rounded text-red-700 overflow-x-auto text-xs font-mono'
                                  },
                                  error?.stack || 'No stack trace available'
                                )
                              ]
                            )
                          ]
                        )
                      ]
                    )
                  )
                )
              );
            }

            return (this.props as any).children;
          }
        }

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
        const errorMessage = error instanceof Error ? error.message : 'Render failed';
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
      <div className={`flex items-center justify-center h-screen bg-gray-50 ${className}`}>
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
      <div className={`flex items-center justify-center h-screen bg-gray-50 ${className}`}>
        <div className="text-center max-w-md mx-auto p-8">
          <FileX className="w-16 h-16 text-gray-400 mx-auto mb-4" />
          <h2 className="text-xl font-semibold text-gray-800 mb-2">No View Found</h2>
          <p className="text-gray-600 mb-6">
            The view at <code className="bg-gray-100 px-2 py-1 rounded text-sm">{viewPath}</code> doesn't exist yet.
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
        {showEditor && (
          <div className="absolute top-4 right-4 z-10">
            <Link
              to={`/editor/pages?file=${encodeURIComponent(viewPath)}`}
              className="inline-flex items-center gap-2 px-4 py-2 bg-white border border-gray-200 rounded-lg hover:bg-gray-50 transition-colors shadow-sm"
            >
              <Edit3 className="w-4 h-4" />
              Edit in Editor
            </Link>
          </div>
        )}
        <div className="flex items-center justify-center h-full">
          <div className="max-w-2xl mx-auto p-8">
            <div className="bg-red-50 border border-red-200 rounded-lg p-6">
              <div className="flex items-start gap-3">
                <AlertCircle className="w-6 h-6 text-red-600 mt-0.5 flex-shrink-0" />
                <div className="flex-1">
                  <h3 className="font-semibold text-red-800 mb-2">View Error</h3>
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
    <div className={className}>
      {showEditor && (
        <div className="absolute top-4 right-4 z-10">
          <Link
            to={`/editor/pages?file=${encodeURIComponent(viewPath)}`}
            className="inline-flex items-center gap-2 px-4 py-2 bg-white border border-gray-200 rounded-lg hover:bg-gray-50 transition-colors shadow-sm"
          >
            <Edit3 className="w-4 h-4" />
            Open Editor
          </Link>
        </div>
      )}
      <div ref={sandboxRef} className="min-h-screen" />
    </div>
  );
};
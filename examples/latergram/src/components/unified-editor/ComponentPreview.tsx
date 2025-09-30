import { AlertCircle, Eye, EyeOff } from 'lucide-react';
import type React from 'react';
import { useEffect, useRef, useState } from 'react';
import { getVFSService } from '../../services/vfs-service';
import { componentRegistry, type ProxiedComponent } from '../ComponentRegistry';
import { useComponentWatcher } from '../hooks/useComponentWatcher';

interface ComponentPreviewProps {
  componentId: string | null;
  className?: string;
}

export const ComponentPreview: React.FC<ComponentPreviewProps> = ({
  componentId,
  className = '',
}) => {
  const sandboxRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<any>(null);
  const [component, setComponent] = useState<ProxiedComponent | null>(null);
  const [renderError, setRenderError] = useState<string | null>(null);
  const [isVisible, setIsVisible] = useState(true);
  const [isRecompiling, setIsRecompiling] = useState(false);

  const { compileAndUpdate } = useComponentWatcher();
  const vfs = getVFSService();

  useEffect(() => {
    if (componentId) {
      const comp = componentRegistry.getComponent(componentId);
      setComponent(comp || null);

      if (comp) {
        const unsubscribe = componentRegistry.onUpdate(componentId, () => {
          const updated = componentRegistry.getComponent(componentId);
          setComponent(updated || null);
        });

        return unsubscribe;
      }
    } else {
      setComponent(null);
    }
  }, [componentId]);

  // Recompile with latest context before previewing
  useEffect(() => {
    const recompileForPreview = async () => {
      if (componentId && vfs.isInitialized()) {
        setIsRecompiling(true);
        setRenderError(null);

        try {
          const comp = componentRegistry.getComponent(componentId);
          if (comp && comp.metadata.filePath) {
            // Read the current source code
            const sourceCode = await vfs.readBytesAsString(
              comp.metadata.filePath
            );

            // Recompile with latest context
            await compileAndUpdate(componentId, sourceCode);
          }
        } catch (error) {
          console.error('Failed to recompile component for preview:', error);
          setRenderError(
            `Recompilation failed: ${error instanceof Error ? error.message : 'Unknown error'}`
          );
        } finally {
          setIsRecompiling(false);
        }
      }
    };

    // Only recompile when componentId changes (i.e., when selecting a different component for preview)
    if (componentId) {
      recompileForPreview();
    }
  }, [componentId, compileAndUpdate, vfs]);

  useEffect(() => {
    if (!component || !sandboxRef.current || !isVisible) {
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

        const element = React.createElement(component.proxy);

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
        console.error('Component render error:', error);
      }
    };

    const timeoutId = setTimeout(renderComponent, 100);

    return () => {
      clearTimeout(timeoutId);
    };
  }, [component, isVisible]);

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

  if (!componentId || !component) {
    return (
      <div
        className={`flex items-center justify-center bg-gray-50 ${className}`}
      >
        <div className="text-center text-gray-500 p-8">
          <Eye className="w-12 h-12 mx-auto mb-3 text-gray-300" />
          <p className="text-sm">Select a component to preview</p>
        </div>
      </div>
    );
  }

  return (
    <div className={`bg-white overflow-hidden ${className}`}>
      {/* <div className="flex items-center justify-between p-3 bg-gray-50">

        <button
          type="button"
          onClick={() => setIsVisible(!isVisible)}
          className="p-1 hover:bg-gray-200 rounded transition-colors"
          title={isVisible ? 'Hide preview' : 'Show preview'}
        >
          {isVisible ? (
            <EyeOff className="w-4 h-4 text-gray-600" />
          ) : (
            <Eye className="w-4 h-4 text-gray-600" />
          )}
        </button>
      </div> */}

      <div className="relative h-full">
        {(component.metadata.status === 'loading' || isRecompiling) && (
          <div className="absolute inset-0 bg-blue-50 bg-opacity-50 flex items-center justify-center z-10">
            <div className="flex items-center gap-2 text-blue-600">
              <div className="w-4 h-4 border-2 border-blue-600 border-t-transparent rounded-full animate-spin" />
              <span className="text-sm">
                {isRecompiling ? 'Recompiling for preview...' : 'Compiling...'}
              </span>
            </div>
          </div>
        )}

        {component.metadata.status === 'error' && (
          <div className="p-4 bg-red-50 border-b border-red-200">
            <div className="flex items-start gap-2">
              <AlertCircle className="w-5 h-5 text-red-600 mt-0.5 flex-shrink-0" />
              <div className="flex-1">
                <h4 className="font-medium text-red-800 mb-1">
                  Compilation Error
                </h4>
                <div className="text-sm text-red-700 font-mono whitespace-pre-wrap">
                  {component.metadata.error || 'Unknown compilation error'}
                </div>
              </div>
            </div>
          </div>
        )}

        {renderError && (
          <div className="p-4 bg-red-50 border-b border-red-200">
            <div className="flex items-start gap-2">
              <AlertCircle className="w-5 h-5 text-red-600 mt-0.5 flex-shrink-0" />
              <div className="flex-1">
                <h4 className="font-medium text-red-800 mb-1">Render Error</h4>
                <div className="text-sm text-red-700 font-mono whitespace-pre-wrap">
                  {renderError}
                </div>
              </div>
            </div>
          </div>
        )}

        <div
          ref={sandboxRef}
          className="p-4 overflow-scroll overflow-y-scroll min-h-full"
          style={{
            display: isVisible ? 'block' : 'none',
            background:
              'linear-gradient(45deg, #f8f9fa 25%, transparent 25%), linear-gradient(-45deg, #f8f9fa 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #f8f9fa 75%), linear-gradient(-45deg, transparent 75%, #f8f9fa 75%)',
            backgroundSize: '20px 20px',
            backgroundPosition: '0 0, 0 10px, 10px -10px, -10px 0px',
          }}
        />

        {!isVisible && (
          <div className="p-8 text-center text-gray-500">
            <EyeOff className="w-8 h-8 mx-auto mb-2 text-gray-300" />
            <p className="text-sm">Preview hidden</p>
          </div>
        )}
      </div>
    </div>
  );
};

import { AlertCircle, Database, Eye, FileText } from 'lucide-react';
import type React from 'react';
import { useEffect, useState } from 'react';
import { componentRegistry } from '../ComponentRegistry';
import { storeRegistry } from '../StoreRegistry';
import { ViewRenderer } from '../ViewRenderer';
import { ComponentPreview } from './ComponentPreview';

interface PreviewPaneProps {
  filePath: string | null;
  fileType: 'component' | 'store' | 'page' | 'generic';
  className?: string;
}

export const PreviewPane: React.FC<PreviewPaneProps> = ({
  filePath,
  fileType,
  className = '',
}) => {
  const [, forceUpdate] = useState(0);

  useEffect(() => {
    const unsubComponents = componentRegistry.onContextUpdate(() => {
      forceUpdate(v => v + 1);
    });

    const unsubStores = storeRegistry.onContextUpdate(() => {
      forceUpdate(v => v + 1);
    });

    return () => {
      unsubComponents();
      unsubStores();
    };
  }, []);

  const getEntityId = () => {
    if (!filePath) return null;

    if (fileType === 'component') {
      const components = componentRegistry.getAllComponents();
      const component = components.find(c => c.metadata.filePath === filePath);
      return component?.id || null;
    }

    if (fileType === 'store') {
      const stores = storeRegistry.getAllStores();
      const store = stores.find(s => s.metadata.filePath === filePath);
      return store?.id || null;
    }

    return null;
  };

  const renderPreview = () => {
    if (!filePath) {
      return (
        <div className="flex items-center justify-center h-full bg-gray-50 p-10 rounded-lg">
          <div className="text-center text-gray-500">
            <Eye className="w-12 h-12 mx-auto mb-3 text-gray-300" />
            <p className="text-sm">Select a file to preview</p>
          </div>
        </div>
      );
    }

    switch (fileType) {
      case 'component': {
        const componentId = getEntityId();
        if (!componentId) {
          return (
            <div className="flex items-center justify-center h-full bg-gray-50">
              <div className="text-center text-gray-500">
                <AlertCircle className="w-12 h-12 mx-auto mb-3 text-gray-300" />
                <p className="text-sm">Component not found in registry</p>
                <p className="text-xs text-gray-400 mt-1">{filePath}</p>
              </div>
            </div>
          );
        }
        return (
          <ComponentPreview componentId={componentId} className="h-full" />
        );
      }

      case 'page': {
        return (
          <div
            className="h-full overflow-auto bg-white relative"
            style={{
              isolation: 'isolate',
              transform: 'translateZ(0)',
              position: 'relative',
              zIndex: 0,
            }}
          >
            <ViewRenderer viewPath={filePath} className="min-h-full" />
          </div>
        );
      }

      case 'store': {
        const storeId = getEntityId();
        const store = storeId ? storeRegistry.getStore(storeId) : null;

        return (
          <div className="h-full bg-gray-50 p-6">
            <div className="bg-white rounded-lg border shadow-sm h-full p-6">
              <div className="flex items-center gap-3 mb-6">
                <Database className="w-6 h-6 text-purple-500" />
                <h3 className="text-lg font-semibold text-gray-800">
                  Store Preview
                </h3>
              </div>

              {store ? (
                <div className="space-y-4">
                  <div className="bg-gray-50 rounded-lg p-4">
                    <h4 className="text-sm font-medium text-gray-700 mb-2">
                      Store Information
                    </h4>
                    <div className="space-y-2 text-sm">
                      <div className="flex justify-between">
                        <span className="text-gray-500">Name:</span>
                        <span className="font-mono text-gray-900">
                          {store.metadata.name}
                        </span>
                      </div>
                      <div className="flex justify-between">
                        <span className="text-gray-500">Status:</span>
                        <span
                          className={`font-medium ${
                            store.metadata.status === 'success'
                              ? 'text-green-600'
                              : store.metadata.status === 'error'
                                ? 'text-red-600'
                                : 'text-blue-600'
                          }`}
                        >
                          {store.metadata.status}
                        </span>
                      </div>
                      <div className="flex justify-between">
                        <span className="text-gray-500">Hook:</span>
                        <span className="font-mono text-gray-900">
                          use{store.metadata.name}
                        </span>
                      </div>
                    </div>
                  </div>

                  {store.metadata.error && (
                    <div className="bg-red-50 border border-red-200 rounded-lg p-4">
                      <div className="flex items-start gap-2">
                        <AlertCircle className="w-5 h-5 text-red-600 mt-0.5" />
                        <div>
                          <h4 className="font-medium text-red-800 mb-1">
                            Store Error
                          </h4>
                          <p className="text-sm text-red-700 font-mono">
                            {store.metadata.error}
                          </p>
                        </div>
                      </div>
                    </div>
                  )}

                  {store.metadata.status === 'success' && (
                    <div className="bg-green-50 border border-green-200 rounded-lg p-4">
                      <p className="text-sm text-green-800">
                        Store compiled successfully and is available for use in
                        components.
                      </p>
                    </div>
                  )}
                </div>
              ) : (
                <div className="flex items-center justify-center h-32">
                  <p className="text-gray-500">No store found for this file</p>
                </div>
              )}
            </div>
          </div>
        );
      }

      case 'generic':
      default:
        return (
          <div className="h-full bg-gray-50 p-6">
            <div className="bg-white rounded-lg border shadow-sm h-full p-6">
              <div className="flex items-center gap-3 mb-6">
                <FileText className="w-6 h-6 text-gray-500" />
                <h3 className="text-lg font-semibold text-gray-800">
                  File Preview
                </h3>
              </div>
              <div className="text-center text-gray-500 mt-8">
                <p className="text-sm">
                  Preview not available for this file type
                </p>
                <p className="text-xs text-gray-400 mt-2">{filePath}</p>
              </div>
            </div>
          </div>
        );
    }
  };

  return <div className={`h-full ${className}`}>{renderPreview()}</div>;
};

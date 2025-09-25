import React, { useState, useEffect, useRef, useCallback } from 'react';
import { Code, Save, AlertCircle, Database, Clock, Circle } from 'lucide-react';
import { storeRegistry, ProxiedStore } from './StoreRegistry';
import { useVFSStore } from './hooks/useVFSStore';
import { useDebounce } from './hooks/useDebounce';
import { compileTSCode } from './utils/tsCompiler';

interface StoreEditorProps {
  storeId: string | null;
  height?: string;
  debounceDelay?: number;
}

type CompilationStatus = 'idle' | 'compiling' | 'success' | 'error';

export const StoreEditor: React.FC<StoreEditorProps> = ({
  storeId,
  debounceDelay = 600,
}) => {
  const [store, setStore] = useState<ProxiedStore | null>(null);
  const [localContent, setLocalContent] = useState<string>('');
  const [isSaving, setIsSaving] = useState(false);
  const [lastSaved, setLastSaved] = useState<Date | null>(null);
  const [compilationStatus, setCompilationStatus] =
    useState<CompilationStatus>('idle');
  const [compilationError, setCompilationError] = useState<string | null>(null);
  const [compilationTime, setCompilationTime] = useState<number | null>(null);
  const [lastCompiledCode, setLastCompiledCode] = useState<string>('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const filePath = store?.metadata.filePath || null;
  const {
    content: vfsContent,
    isLoading,
    error: vfsError,
    updateContent,
  } = useVFSStore(filePath);
  const debouncedContent = useDebounce(localContent, debounceDelay);

  useEffect(() => {
    if (storeId) {
      const storeItem = storeRegistry.getStore(storeId);
      setStore(storeItem || null);

      if (storeItem) {
        const unsubscribe = storeRegistry.onUpdate(storeId, () => {
          const updated = storeRegistry.getStore(storeId);
          setStore(updated || null);
        });

        return unsubscribe;
      }
    } else {
      setStore(null);
    }
  }, [storeId]);

  useEffect(() => {
    if (vfsContent !== localContent && vfsContent !== '') {
      setLocalContent(vfsContent);
    }
  }, [vfsContent]);

  const compileStore = useCallback(
    async (code: string, forceRecompile = false) => {
      if (
        !store ||
        !code.trim() ||
        (!forceRecompile && code === lastCompiledCode)
      ) {
        return;
      }

      const startTime = performance.now();
      setCompilationStatus('compiling');
      setCompilationError(null);

      try {
        // Compile the store using the shared compiler utility
        const result = await compileTSCode(code, {
          moduleType: 'CommonJS',
          outputFormat: 'module',
          excludeStoreId: store.id,
        });

        if (result.success && typeof result.output === 'function') {
          // Update the store in the registry
          storeRegistry.update(store.id, result.output);

          const endTime = performance.now();
          setCompilationTime(Math.round(endTime - startTime));
          setCompilationStatus('success');
          setLastCompiledCode(code);
        } else {
          throw new Error(result.error || 'Unknown compilation error');
        }
      } catch (error) {
        const errorMessage =
          error instanceof Error ? error.message : 'Unknown compilation error';
        setCompilationStatus('error');
        setCompilationError(errorMessage);

        // Update store with error status
        if (store) {
          storeRegistry.update(store.id, null, 'error', errorMessage);
        }
      }
    },
    [store, lastCompiledCode]
  );

  const saveContent = async () => {
    if (!store || !debouncedContent) return;

    setIsSaving(true);
    try {
      await updateContent(debouncedContent);
      setLastSaved(new Date());
    } catch (error) {
      console.error('Failed to save store content:', error);
    } finally {
      setIsSaving(false);
    }
  };

  // Trigger compilation when content changes
  useEffect(() => {
    if (debouncedContent && debouncedContent !== vfsContent && store) {
      saveContent();
      compileStore(debouncedContent);
    }
  }, [debouncedContent, compileStore, vfsContent, store]);

  // Load TypeScript compiler if not available and compile initial content
  useEffect(() => {
    const loadTypeScript = async () => {
      if (!(window as any).ts) {
        const script = document.createElement('script');
        script.src = '/typescript.js';
        script.onload = () => {
          if (localContent.trim() && store) {
            compileStore(localContent, true);
          }
        };
        document.head.appendChild(script);
      } else if (localContent.trim() && store) {
        compileStore(localContent, true);
      }
    };
    loadTypeScript();
  }, [localContent, store, compileStore]);

  const handleContentChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setLocalContent(e.target.value);
  };

  const formatTime = (date: Date) => {
    return date.toLocaleTimeString([], {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  };

  if (!storeId || !store) {
    return (
      <div className="w-full bg-white rounded-lg border shadow-sm">
        <div className="flex items-center justify-center h-96 text-gray-500">
          <div className="text-center">
            <Database className="w-12 h-12 mx-auto mb-3 text-gray-300" />
            <p className="text-sm">Select a store to edit</p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="w-full h-full bg-white rounded-lg border shadow-sm flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between p-4 border-b bg-gray-50 flex-shrink-0">
        <div className="flex items-center gap-3">
          <Code className="w-5 h-5 text-purple-500" />
          <div className="flex flex-col">
            <h3 className="font-semibold text-gray-800">
              {store.metadata.name}
            </h3>
            <p className="text-xs text-gray-500">{store.metadata.filePath}</p>
          </div>
        </div>

        <div className="flex items-center gap-4 text-sm">
          {/* Save Status */}
          {isSaving ? (
            <div className="flex items-center gap-2 text-blue-600">
              <div className="w-4 h-4 border-2 border-blue-600 border-t-transparent rounded-full animate-spin" />
              <span>Saving...</span>
            </div>
          ) : lastSaved ? (
            <div className="flex items-center gap-2 text-green-600">
              <Save className="w-4 h-4" />
              <span>Saved at {formatTime(lastSaved)}</span>
            </div>
          ) : (
            <div className="flex items-center gap-2 text-gray-500">
              <Clock className="w-4 h-4" />
              <span>Auto-save</span>
            </div>
          )}

          {/* Compilation Status */}
          {compilationStatus === 'compiling' ? (
            <div className="flex items-center gap-2 text-blue-600">
              <div className="w-4 h-4 border-2 border-blue-600 border-t-transparent rounded-full animate-spin" />
              <span>Compiling...</span>
            </div>
          ) : compilationStatus === 'success' ? (
            <div className="flex items-center gap-2 text-green-600">
              <Circle className="w-4 h-4 fill-green-500 text-green-500" />
              <span>
                Compiled {compilationTime ? `in ${compilationTime}ms` : ''}
              </span>
            </div>
          ) : compilationStatus === 'error' ? (
            <div className="flex items-center gap-2 text-red-600">
              <Circle className="w-4 h-4 fill-red-500 text-red-500" />
              <span>Compilation error</span>
            </div>
          ) : null}
        </div>
      </div>

      {/* Store Status */}
      {store.metadata.status === 'error' && store.metadata.error && (
        <div className="p-4 bg-red-50 border-b border-red-200">
          <div className="flex items-start gap-2">
            <AlertCircle className="w-5 h-5 text-red-600 mt-0.5" />
            <div className="flex-1">
              <h4 className="font-medium text-red-800 mb-1">Store Error</h4>
              <div className="text-sm text-red-700 font-mono whitespace-pre-wrap">
                {store.metadata.error}
              </div>
            </div>
          </div>
        </div>
      )}

      {/* VFS Error */}
      {vfsError && (
        <div className="p-4 bg-red-50 border-b border-red-200">
          <div className="flex items-start gap-2">
            <AlertCircle className="w-5 h-5 text-red-600 mt-0.5" />
            <div className="flex-1">
              <h4 className="font-medium text-red-800 mb-1">
                File System Error
              </h4>
              <div className="text-sm text-red-700">{vfsError}</div>
            </div>
          </div>
        </div>
      )}

      {/* Compilation Error */}
      {compilationError && (
        <div className="p-4 bg-red-50 border-b border-red-200">
          <div className="flex items-start gap-2">
            <AlertCircle className="w-5 h-5 text-red-600 mt-0.5" />
            <div className="flex-1">
              <h4 className="font-medium text-red-800 mb-1">
                Store Compilation Error
              </h4>
              <div className="text-sm text-red-700 font-mono whitespace-pre-wrap">
                {compilationError}
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Editor */}
      <div className="relative flex-1 flex flex-col">
        {isLoading && (
          <div className="absolute inset-0 bg-gray-50 bg-opacity-75 flex items-center justify-center z-10">
            <div className="flex items-center gap-2 text-gray-600">
              <div className="w-5 h-5 border-2 border-gray-600 border-t-transparent rounded-full animate-spin" />
              <span>Loading store...</span>
            </div>
          </div>
        )}

        <textarea
          ref={textareaRef}
          value={localContent}
          onChange={handleContentChange}
          placeholder={`// Create your ${store.metadata.name} store here...
// 'create' and 'sync' are available in the compilation context
// All stores are automatically available in components as hooks

interface ${store.metadata.name}State {
  // Define your state interface here
}

return create<${store.metadata.name}State>()(
  sync(
    (set) => ({
      // Your state here
    }),
    { path: '/src/stores/${store.metadata.name.toLowerCase()}-store.json' }
  )
);`}
          className="w-full h-full p-4 font-mono text-sm border-0 resize-none focus:outline-none bg-gray-50"
          spellCheck={false}
          disabled={isLoading}
        />

        {isSaving && (
          <div className="absolute top-2 right-2 px-2 py-1 bg-blue-100 text-blue-700 text-xs rounded">
            Saving...
          </div>
        )}
      </div>

      {/* Info Panel */}
      <div className="border-t bg-gray-50 p-4 flex-shrink-0">
        <div className="text-sm text-gray-600">
          <div className="flex items-center gap-4 flex-wrap">
            <span>
              <strong>Hook:</strong> {store.metadata.name}
            </span>
            <span>
              <strong>Status:</strong>{' '}
              <span
                className={
                  store.metadata.status === 'success'
                    ? 'text-green-600'
                    : store.metadata.status === 'error'
                      ? 'text-red-600'
                      : 'text-blue-600'
                }
              >
                {store.metadata.status}
              </span>
            </span>
            <span>
              <strong>Modified:</strong>{' '}
              {store.metadata.modified.toLocaleString()}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
};

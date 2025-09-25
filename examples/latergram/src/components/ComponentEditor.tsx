import React, { useState, useEffect, useRef, useCallback } from 'react';
import { Code, Save, AlertCircle, FileText, Clock, RefreshCw } from 'lucide-react';
import { componentRegistry, ProxiedComponent } from './ComponentRegistry';
import { useVFSComponent } from './hooks/useVFSComponent';
import { useAutoSave } from './shared/hooks/useAutoSave';

interface ComponentEditorProps {
  componentId: string | null;
  height?: string;
  debounceDelay?: number;
}

export const ComponentEditor: React.FC<ComponentEditorProps> = ({
  componentId,
  height = '400px',
  debounceDelay = 600,
}) => {
  const [component, setComponent] = useState<ProxiedComponent | null>(null);
  const [localContent, setLocalContent] = useState<string>('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const filePath = component?.metadata.filePath || null;
  const {
    content: vfsContent,
    isLoading,
    error: vfsError,
    updateContent,
  } = useVFSComponent(filePath);

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

  useEffect(() => {
    if (vfsContent !== localContent && vfsContent !== '') {
      setLocalContent(vfsContent);
    }
  }, [vfsContent]);

  // Save content function for auto-save
  const saveContent = useCallback(async (content: string) => {
    if (!component || !content) return;

    try {
      await updateContent(content);
      return true;
    } catch (error) {
      console.error('Failed to save content:', error);
      throw error;
    }
  }, [component, updateContent]);

  // Use auto-save hook
  const { isSaving, lastSaved, hasChanges } = useAutoSave({
    content: localContent,
    onSave: saveContent,
    debounceMs: debounceDelay,
    enabled: !!component,
  });

  const handleContentChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setLocalContent(e.target.value);
  };

  if (!componentId || !component) {
    return (
      <div className="w-full bg-white rounded-lg border shadow-sm">
        <div className="flex items-center justify-center h-96 text-gray-500">
          <div className="text-center">
            <FileText className="w-12 h-12 mx-auto mb-3 text-gray-300" />
            <p className="text-sm">Select a component to edit</p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="w-full bg-white rounded-lg border shadow-sm h-full flex flex-col">
      <div className="flex items-center justify-between p-4 border-b bg-gray-50">
        <div className="flex items-center gap-3">
          <Code className="w-5 h-5 text-blue-500" />
          <h3 className="font-semibold text-gray-800">
            {component.metadata.name}
          </h3>

          <div className="text-xs text-gray-500">
            {component.metadata.filePath}
          </div>
        </div>

        <div className="flex items-center gap-3">
          {isSaving && (
            <span className="text-xs text-blue-600 flex items-center gap-1">
              <RefreshCw className="w-3 h-3 animate-spin" />
              Saving...
            </span>
          )}
          {!isSaving && lastSaved && (
            <span className="text-xs text-gray-500 flex items-center gap-1">
              <Clock className="w-3 h-3" />
              Auto-saved
            </span>
          )}
          {hasChanges && !isSaving && (
            <span className="text-xs text-amber-600">
              • Unsaved
            </span>
          )}

          <div className="flex items-center gap-2">
            {component.metadata.status === 'loading' && (
              <Clock className="w-4 h-4 text-blue-500" />
            )}
            {component.metadata.status === 'error' && (
              <AlertCircle className="w-4 h-4 text-red-500" />
            )}
            {component.metadata.status === 'success' && (
              <div className="w-4 h-4 bg-green-500 rounded-full" />
            )}
            <span
              className={`text-sm ${
                component.metadata.status === 'error'
                  ? 'text-red-600'
                  : component.metadata.status === 'loading'
                    ? 'text-blue-600'
                    : 'text-green-600'
              }`}
            >
              {component.metadata.status === 'error'
                ? 'Error'
                : component.metadata.status === 'loading'
                  ? 'Compiling...'
                  : 'Ready'}
            </span>
          </div>
        </div>
      </div>

      <div className="relative flex flex-grow">
        {isLoading && (
          <div className="absolute top-2 right-2 px-2 py-1 bg-blue-100 text-blue-700 text-xs rounded z-10">
            Loading...
          </div>
        )}

        <textarea
          ref={textareaRef}
          value={localContent}
          onChange={handleContentChange}
          placeholder="Write TypeScript component code here..."
          className="w-full h-full p-4 font-mono text-sm border-0 resize-none focus:outline-none bg-gray-50"
          style={{ height, minHeight: '200px' }}
          spellCheck={false}
          disabled={isLoading}
        />
      </div>

      {(vfsError || component.metadata.error) && (
        <div className="p-4 bg-red-50 border-t border-red-200">
          <div className="flex items-start gap-2">
            <AlertCircle className="w-5 h-5 text-red-600 mt-0.5" />
            <div className="flex-1">
              <h4 className="font-medium text-red-800 mb-1">
                {vfsError ? 'VFS Error' : 'Compilation Error'}
              </h4>
              <div className="text-sm text-red-700 font-mono whitespace-pre-wrap">
                {vfsError || component.metadata.error}
              </div>
            </div>
          </div>
        </div>
      )}

      <div className="px-4 py-2 border-t bg-gray-50 text-xs text-gray-500">
        <div className="flex justify-between items-center">
          <span>
            Changes auto-save • Last modified:{' '}
            {formatTime(component.metadata.modified)}
          </span>
          <span>
            Lines: {localContent.split('\n').length} • Characters:{' '}
            {localContent.length}
          </span>
        </div>
      </div>
    </div>
  );
};

import React, { useState, useEffect, useCallback, useRef } from 'react';
import { Eye, Code, Save, Clock, RefreshCw, AlertCircle } from 'lucide-react';
import { MonacoCodeEditor } from './MonacoCodeEditor';
import { PreviewPane } from './PreviewPane';
import { FileTree } from '../filetree/FileTree';
import { getVFSService } from '../../services/vfs-service';
import { useAutoSave } from '../shared/hooks/useAutoSave';
import { typeScriptValidator } from '../../lib/typescript-validator';
import { componentRegistry } from '../ComponentRegistry';
import { storeRegistry } from '../StoreRegistry';
import { useComponentWatcher } from '../hooks/useComponentWatcher';

interface UnifiedEditorProps {
  fileFilter?: string;
  editorOnly?: boolean;
  defaultTab?: 'preview' | 'editor';
  onFileChange?: (path: string) => void;
  height?: string;
  debounceDelay?: number;
}

type FileType = 'component' | 'store' | 'page' | 'generic';

const getFileType = (filePath: string): FileType => {
  if (filePath.startsWith('/src/components/')) return 'component';
  if (filePath.startsWith('/src/stores/')) return 'store';
  if (filePath.startsWith('/src/views/')) return 'page';
  return 'generic';
};

export const UnifiedEditor: React.FC<UnifiedEditorProps> = ({
  fileFilter,
  editorOnly = false,
  defaultTab = 'preview',
  onFileChange,
  height = '100%',
  debounceDelay = 1000,
}) => {
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [fileContent, setFileContent] = useState<string>('');
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'preview' | 'editor'>(editorOnly ? 'editor' : defaultTab);
  const [validationErrors, setValidationErrors] = useState<any[]>([]);
  const [compilationStatus, setCompilationStatus] = useState<'idle' | 'compiling' | 'success' | 'error'>('idle');

  const vfs = getVFSService();
  const { watchComponent, compileAndUpdate } = useComponentWatcher();

  const fileType = selectedFile ? getFileType(selectedFile) : 'generic';
  const watchIdRef = useRef<string | null>(null);

  // Load file content
  const loadFile = useCallback(async (filePath: string) => {
    if (!vfs.isInitialized()) return;

    setIsLoading(true);
    setError(null);

    try {
      const exists = await vfs.exists(filePath);
      if (!exists) {
        setFileContent('');
        setError(`File not found: ${filePath}`);
      } else {
        const content = await vfs.readBytesAsString(filePath);
        setFileContent(content);

        // Validate TypeScript if it's a TS/TSX file
        if (filePath.endsWith('.ts') || filePath.endsWith('.tsx')) {
          const validation = typeScriptValidator.validateSyntax(content);
          if (!validation.valid && validation.diagnostics) {
            setValidationErrors(validation.diagnostics.map(d => ({
              line: d.line || 1,
              column: d.column || 1,
              message: d.message,
              severity: d.category === 'error' ? 'error' : d.category === 'warning' ? 'warning' : 'info',
            })));
          } else {
            setValidationErrors([]);
          }
        }
      }
    } catch (error) {
      console.error('Failed to load file:', error);
      setError(`Failed to load file: ${error instanceof Error ? error.message : 'Unknown error'}`);
      setFileContent('');
    } finally {
      setIsLoading(false);
    }
  }, [vfs]);

  // Handle file selection
  const handleFileSelect = useCallback(async (filePath: string) => {
    // Unwatch previous file if any
    if (watchIdRef.current) {
      try {
        await vfs.unwatchFile(watchIdRef.current);
      } catch (error) {
        console.error('Failed to unwatch previous file:', error);
      }
      watchIdRef.current = null;
    }

    setSelectedFile(filePath);
    onFileChange?.(filePath);
    loadFile(filePath);

    // Watch the new file for changes
    try {
      const watchId = await vfs.watchFile(filePath, async (changeData) => {
        console.log(`File ${filePath} changed:`, changeData);

        // Check if file was deleted
        const exists = await vfs.exists(filePath);
        if (!exists) {
          // File was deleted
          setSelectedFile(null);
          setFileContent('');
          setError(`File ${filePath} was deleted`);
          setValidationErrors([]);

          // Unwatch the deleted file
          if (watchIdRef.current) {
            try {
              await vfs.unwatchFile(watchIdRef.current);
            } catch (error) {
              console.error('Failed to unwatch deleted file:', error);
            }
            watchIdRef.current = null;
          }
        } else {
          // File was modified, reload it
          loadFile(filePath);
        }
      });

      watchIdRef.current = watchId;
    } catch (error) {
      console.error('Failed to watch file:', error);
    }
  }, [vfs, loadFile, onFileChange]);

  // Save file content
  const saveFile = useCallback(async (content: string) => {
    if (!vfs.isInitialized() || !selectedFile) return;

    setCompilationStatus('compiling');
    try {
      // Save to VFS
      await vfs.writeStringAsBytes(selectedFile, content, false);

      // Handle specific file types
      const type = getFileType(selectedFile);

      if (type === 'component') {
        // Find or create component in registry
        let componentId: string | null = null;
        const components = componentRegistry.getAllComponents();
        const existingComponent = components.find(c => c.metadata.filePath === selectedFile);

        if (existingComponent) {
          componentId = existingComponent.id;
        } else {
          // Extract component name from file path
          const fileName = selectedFile.split('/').pop()?.replace('.tsx', '') || 'Component';
          componentId = componentRegistry.createComponent(fileName);
          const component = componentRegistry.getComponent(componentId);
          if (component) {
            // Watch the component file
            await watchComponent(componentId, selectedFile);
          }
        }

        // Compile and update component
        if (componentId) {
          await compileAndUpdate(componentId, content);
        }
      } else if (type === 'store') {
        // Handle store compilation
        const stores = storeRegistry.getAllStores();
        const existingStore = stores.find(s => s.metadata.filePath === selectedFile);

        if (!existingStore) {
          // Extract store name from file path
          const fileName = selectedFile.split('/').pop()?.replace('.ts', '').replace('.tsx', '') || 'Store';
          const storeName = fileName.replace(/-store$/i, '').replace(/store$/i, '');
          const capitalizedName = storeName.charAt(0).toUpperCase() + storeName.slice(1);
          storeRegistry.createStore(capitalizedName);
        }
      }

      // Validate TypeScript
      if (selectedFile.endsWith('.ts') || selectedFile.endsWith('.tsx')) {
        const validation = typeScriptValidator.validateSyntax(content);
        if (!validation.valid && validation.diagnostics) {
          setValidationErrors(validation.diagnostics.map(d => ({
            line: d.line || 1,
            column: d.column || 1,
            message: d.message,
            severity: d.category === 'error' ? 'error' : d.category === 'warning' ? 'warning' : 'info',
          })));
          setCompilationStatus('error');
        } else {
          setValidationErrors([]);
          setCompilationStatus('success');
        }
      } else {
        setCompilationStatus('success');
      }

      return true;
    } catch (error) {
      console.error('Failed to save file:', error);
      setError(`Failed to save: ${error instanceof Error ? error.message : 'Unknown error'}`);
      setCompilationStatus('error');
      throw error;
    }
  }, [vfs, selectedFile, watchComponent, compileAndUpdate]);

  // Use auto-save hook
  const { isSaving, lastSaved, hasChanges } = useAutoSave({
    content: fileContent,
    onSave: saveFile,
    debounceMs: debounceDelay,
    enabled: !!selectedFile,
  });

  // Handle manual save
  const handleManualSave = useCallback(() => {
    if (selectedFile && fileContent) {
      saveFile(fileContent);
    }
  }, [selectedFile, fileContent, saveFile]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      // Unwatch file when component unmounts
      if (watchIdRef.current) {
        vfs.unwatchFile(watchIdRef.current).catch(error => {
          console.error('Failed to unwatch file on unmount:', error);
        });
      }
    };
  }, [vfs]);

  return (
    <div className="flex h-full bg-gray-100 overflow-hidden">
      {/* File Browser */}
      <div className="w-64 border-r border-gray-200 bg-white">
        <FileTree
          rootPath={fileFilter || '/'}
          onFileSelect={handleFileSelect}
          onFileDelete={(deletedPath) => {
            // If the deleted file is currently selected, clear the editor
            if (deletedPath === selectedFile) {
              setSelectedFile(null);
              setFileContent('');
              setError(`File ${deletedPath} was deleted`);
              setValidationErrors([]);

              // Unwatch the deleted file
              if (watchIdRef.current) {
                vfs.unwatchFile(watchIdRef.current).catch(error => {
                  console.error('Failed to unwatch deleted file:', error);
                });
                watchIdRef.current = null;
              }
            }
          }}
          className="h-full"
        />
      </div>

      {/* Main Editor Area */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* Header */}
        <div className="bg-white border-b border-gray-200 px-6 py-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-4">
              <h2 className="text-lg font-semibold text-gray-800">
                {selectedFile ? selectedFile.split('/').pop() : 'Editor'}
              </h2>
              {selectedFile && (
                <span className="text-sm text-gray-500">{selectedFile}</span>
              )}
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
                <span className="text-xs text-amber-600">• Unsaved</span>
              )}

              {compilationStatus === 'compiling' && (
                <span className="text-xs text-blue-600">Compiling...</span>
              )}
              {compilationStatus === 'success' && (
                <span className="text-xs text-green-600">✓ Compiled</span>
              )}
              {compilationStatus === 'error' && (
                <span className="text-xs text-red-600">✗ Error</span>
              )}

              <button
                type="button"
                onClick={handleManualSave}
                className="px-3 py-1 text-sm bg-blue-500 text-white rounded hover:bg-blue-600 transition-colors flex items-center gap-1"
                disabled={!hasChanges || isSaving}
              >
                <Save className="w-4 h-4" />
                Save
              </button>
            </div>
          </div>
        </div>

        {/* Tab Bar */}
        {!editorOnly && (
          <div className="bg-white border-b border-gray-200 px-6">
            <div className="flex gap-1">
              <button
                type="button"
                onClick={() => setActiveTab('preview')}
                className={`px-4 py-2 font-medium text-sm border-b-2 transition-colors flex items-center gap-2 ${
                  activeTab === 'preview'
                    ? 'border-blue-500 text-blue-600'
                    : 'border-transparent text-gray-500 hover:text-gray-700'
                }`}
              >
                <Eye className="w-4 h-4" />
                Preview
              </button>
              <button
                type="button"
                onClick={() => setActiveTab('editor')}
                className={`px-4 py-2 font-medium text-sm border-b-2 transition-colors flex items-center gap-2 ${
                  activeTab === 'editor'
                    ? 'border-blue-500 text-blue-600'
                    : 'border-transparent text-gray-500 hover:text-gray-700'
                }`}
              >
                <Code className="w-4 h-4" />
                Editor
              </button>
            </div>
          </div>
        )}

        {/* Error Display */}
        {error && (
          <div className="bg-red-50 border-b border-red-200 px-6 py-3">
            <div className="flex items-center gap-2 text-sm text-red-700">
              <AlertCircle className="w-4 h-4" />
              {error}
            </div>
          </div>
        )}

        {/* Content Area */}
        <div className="flex-1 overflow-hidden">
          {isLoading ? (
            <div className="flex items-center justify-center h-full bg-gray-50">
              <div className="text-center">
                <RefreshCw className="w-8 h-8 animate-spin text-blue-500 mx-auto mb-4" />
                <p className="text-gray-600">Loading file...</p>
              </div>
            </div>
          ) : activeTab === 'editor' || editorOnly ? (
            <MonacoCodeEditor
              value={fileContent}
              onChange={setFileContent}
              language={selectedFile?.endsWith('.tsx') ? 'typescript' : 'typescript'}
              height={height}
              onSave={handleManualSave}
              errors={validationErrors}
              filePath={selectedFile || undefined}
            />
          ) : (
            <PreviewPane
              filePath={selectedFile}
              fileType={fileType}
              className="h-full"
            />
          )}
        </div>
      </div>
    </div>
  );
};
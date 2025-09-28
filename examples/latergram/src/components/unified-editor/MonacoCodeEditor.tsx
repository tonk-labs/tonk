import React, { useRef, useCallback } from 'react';
import Editor, { Monaco, OnChange, OnMount } from '@monaco-editor/react';
import { editor } from 'monaco-editor';

interface MonacoCodeEditorProps {
  value: string;
  onChange: (value: string) => void;
  language?: string;
  height?: string;
  readOnly?: boolean;
  onSave?: () => void;
  errors?: Array<{
    line: number;
    column: number;
    message: string;
    severity: 'error' | 'warning' | 'info';
  }>;
}

export const MonacoCodeEditor: React.FC<MonacoCodeEditorProps> = ({
  value,
  onChange,
  language = 'typescript',
  height = '100%',
  readOnly = false,
  onSave,
  errors = [],
}) => {
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<Monaco | null>(null);

  const handleEditorDidMount: OnMount = useCallback((editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;

    // Configure TypeScript compiler options
    monaco.languages.typescript.typescriptDefaults.setCompilerOptions({
      target: monaco.languages.typescript.ScriptTarget.Latest,
      module: monaco.languages.typescript.ModuleKind.ESNext,
      jsx: monaco.languages.typescript.JsxEmit.React,
      allowNonTsExtensions: true,
      moduleResolution: monaco.languages.typescript.ModuleResolutionKind.NodeJs,
      allowSyntheticDefaultImports: true,
      esModuleInterop: true,
      noEmit: true,
      skipLibCheck: true,
      lib: ['es2020', 'dom'],
    });

    // Add React types
    monaco.languages.typescript.typescriptDefaults.addExtraLib(
      `declare module 'react' {
        export = React;
        export as namespace React;
      }`,
      'file:///node_modules/@types/react/index.d.ts'
    );

    // Configure editor options
    editor.updateOptions({
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      wordWrap: 'on',
      automaticLayout: true,
      formatOnPaste: true,
      formatOnType: true,
      suggestOnTriggerCharacters: true,
      quickSuggestions: {
        other: true,
        comments: false,
        strings: true,
      },
      fontSize: 14,
      lineNumbers: 'on',
      renderWhitespace: 'selection',
      bracketPairColorization: {
        enabled: true,
      },
      'semanticHighlighting.enabled': true,
    });

    // Add save keyboard shortcut
    if (onSave) {
      editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
        onSave();
      });
    }

    // Set up error markers if any
    updateErrorMarkers(monaco, editor);
  }, [onSave]);

  const updateErrorMarkers = useCallback((monaco: Monaco, editor: editor.IStandaloneCodeEditor) => {
    const model = editor.getModel();
    if (!model) return;

    const markers = errors.map(error => ({
      severity: error.severity === 'error'
        ? monaco.MarkerSeverity.Error
        : error.severity === 'warning'
        ? monaco.MarkerSeverity.Warning
        : monaco.MarkerSeverity.Info,
      startLineNumber: error.line,
      startColumn: error.column,
      endLineNumber: error.line,
      endColumn: error.column + 1,
      message: error.message,
    }));

    monaco.editor.setModelMarkers(model, 'typescript', markers);
  }, [errors]);

  // Update error markers when errors change
  React.useEffect(() => {
    if (editorRef.current && monacoRef.current) {
      updateErrorMarkers(monacoRef.current, editorRef.current);
    }
  }, [errors, updateErrorMarkers]);

  const handleChange: OnChange = useCallback((value) => {
    onChange(value || '');
  }, [onChange]);

  return (
    <div className="w-full h-full border border-gray-200 rounded-lg overflow-hidden">
      <Editor
        height={height}
        defaultLanguage={language}
        language={language}
        value={value}
        onChange={handleChange}
        onMount={handleEditorDidMount}
        theme="vs-dark"
        options={{
          readOnly,
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          wordWrap: 'on',
          automaticLayout: true,
        }}
        loading={
          <div className="flex items-center justify-center h-full bg-gray-900 text-white">
            <div className="flex items-center gap-2">
              <div className="w-4 h-4 border-2 border-blue-500 border-t-transparent rounded-full animate-spin" />
              <span>Loading editor...</span>
            </div>
          </div>
        }
      />
    </div>
  );
};
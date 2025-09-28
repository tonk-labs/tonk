import React, { useRef, useCallback, useEffect } from 'react';
import Editor, { OnChange, OnMount } from '@monaco-editor/react';
import { buildAvailablePackages } from '../contextBuilder';
import { componentRegistry } from '../ComponentRegistry';
import { storeRegistry } from '../StoreRegistry';

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
  filePath?: string;
}

// Generate TypeScript declarations for Tonk globals
const generateMonacoTypes = (): string => {
  const packages = buildAvailablePackages();

  let declarations = `
// React core types
declare namespace React {
  type FC<P = {}> = FunctionComponent<P>;
  type FunctionComponent<P = {}> = (props: P) => ReactElement | null;
  type ComponentType<P = {}> = FC<P> | ComponentClass<P>;
  type ReactNode = ReactElement | string | number | boolean | null | undefined;
  type ReactElement = any;
  type ComponentClass<P = {}> = any;
  type RefObject<T> = { current: T | null };
  type MutableRefObject<T> = { current: T };
  type SetStateAction<S> = S | ((prevState: S) => S);
  type Dispatch<A> = (value: A) => void;
  type EffectCallback = () => (void | (() => void));
  type DependencyList = ReadonlyArray<any>;
  type Reducer<S, A> = (prevState: S, action: A) => S;
  type Context<T> = any;

  interface CSSProperties {
    [key: string]: any;
  }

  function createElement(type: any, props?: any, ...children: any[]): ReactElement;
}

declare const React: {
  createElement: typeof React.createElement;
  Fragment: React.ComponentType;
  useState: <S>(initialState: S | (() => S)) => [S, React.Dispatch<React.SetStateAction<S>>];
  useEffect: (effect: React.EffectCallback, deps?: React.DependencyList) => void;
  useCallback: <T extends (...args: any[]) => any>(callback: T, deps: React.DependencyList) => T;
  useMemo: <T>(factory: () => T, deps: React.DependencyList) => T;
  useRef: <T>(initialValue: T) => React.MutableRefObject<T>;
  useReducer: <R extends React.Reducer<any, any>>(
    reducer: R,
    initialState: Parameters<R>[0]
  ) => [Parameters<R>[0], React.Dispatch<Parameters<R>[1]>];
  useContext: <T>(context: React.Context<T>) => T;
};

declare const useState: typeof React.useState;
declare const useEffect: typeof React.useEffect;
declare const useCallback: typeof React.useCallback;
declare const useMemo: typeof React.useMemo;
declare const useRef: typeof React.useRef;
declare const useReducer: typeof React.useReducer;
declare const useContext: typeof React.useContext;
declare const Fragment: typeof React.Fragment;

// Zustand imports
declare const create: any;
declare const sync: any;

// Export default is required for Tonk components
declare const exports: { default?: React.ComponentType<any> };
declare const module: { exports: typeof exports };
`;

  // Add component declarations
  componentRegistry.getAllComponents().forEach(comp => {
    const name = comp.metadata.name.replace(/[^a-zA-Z0-9]/g, '') || 'UnnamedComponent';
    if (name !== 'UnnamedComponent') {
      declarations += `declare const ${name}: React.ComponentType<any>;\n`;
    }
  });

  // Add store declarations
  storeRegistry.getAllStores().forEach(store => {
    const name = store.metadata.name.replace(/[^a-zA-Z0-9]/g, '') || 'UnnamedStore';
    if (name !== 'UnnamedStore') {
      declarations += `declare const ${name}: any;\n`;
    }
  });

  return declarations;
};

export const MonacoCodeEditor: React.FC<MonacoCodeEditorProps> = ({
  value,
  onChange,
  language = 'typescript',
  height = '100%',
  readOnly = false,
  onSave,
  errors = [],
  filePath,
}) => {
  const editorRef = useRef<any>(null);
  const monacoRef = useRef<any>(null);

  const handleEditorDidMount: OnMount = useCallback((editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;

    // Add save keyboard shortcut
    if (onSave) {
      editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
        onSave();
      });
    }

    // Configure TypeScript compiler options for Tonk
    monaco.languages.typescript.typescriptDefaults.setCompilerOptions({
      target: monaco.languages.typescript.ScriptTarget.ES2020,
      module: monaco.languages.typescript.ModuleKind.CommonJS,
      jsx: monaco.languages.typescript.JsxEmit.React,
      esModuleInterop: true,
      skipLibCheck: true,
      noEmit: true,
      allowSyntheticDefaultImports: true,
      strict: false,
      lib: ['es2020', 'dom'],
    });

    // Add Tonk global type declarations
    const tonkTypes = generateMonacoTypes();
    monaco.languages.typescript.typescriptDefaults.addExtraLib(
      tonkTypes,
      'tonk-globals.d.ts'
    );

    // Update markers with current errors
    if (errors.length > 0 && filePath) {
      const model = editor.getModel();
      if (model) {
        const markers = errors.map(err => ({
          severity: err.severity === 'error'
            ? monaco.MarkerSeverity.Error
            : err.severity === 'warning'
              ? monaco.MarkerSeverity.Warning
              : monaco.MarkerSeverity.Info,
          startLineNumber: err.line,
          startColumn: err.column,
          endLineNumber: err.line,
          endColumn: err.column + 1,
          message: err.message,
        }));
        monaco.editor.setModelMarkers(model, 'tonk', markers);
      }
    }
  }, [onSave, errors, filePath]);

  // Update error markers when errors change
  useEffect(() => {
    if (editorRef.current && monacoRef.current && filePath) {
      const model = editorRef.current.getModel();
      if (model) {
        const markers = errors.map(err => ({
          severity: err.severity === 'error'
            ? monacoRef.current.MarkerSeverity.Error
            : err.severity === 'warning'
              ? monacoRef.current.MarkerSeverity.Warning
              : monacoRef.current.MarkerSeverity.Info,
          startLineNumber: err.line,
          startColumn: err.column,
          endLineNumber: err.line,
          endColumn: err.column + 1,
          message: err.message,
        }));
        monacoRef.current.editor.setModelMarkers(model, 'tonk', markers);
      }
    }
  }, [errors, filePath]);

  // Update TypeScript declarations when components or stores change
  useEffect(() => {
    if (monacoRef.current) {
      const tonkTypes = generateMonacoTypes();
      monacoRef.current.languages.typescript.typescriptDefaults.addExtraLib(
        tonkTypes,
        'tonk-globals.d.ts'
      );
    }

    // Subscribe to registry changes
    const unsubComponents = componentRegistry.onContextUpdate(() => {
      if (monacoRef.current) {
        const tonkTypes = generateMonacoTypes();
        monacoRef.current.languages.typescript.typescriptDefaults.addExtraLib(
          tonkTypes,
          'tonk-globals.d.ts'
        );
      }
    });

    const unsubStores = storeRegistry.onContextUpdate(() => {
      if (monacoRef.current) {
        const tonkTypes = generateMonacoTypes();
        monacoRef.current.languages.typescript.typescriptDefaults.addExtraLib(
          tonkTypes,
          'tonk-globals.d.ts'
        );
      }
    });

    return () => {
      unsubComponents();
      unsubStores();
    };
  }, []);

  const handleChange: OnChange = useCallback((value) => {
    onChange(value || '');
  }, [onChange]);

  return (
    <div className="w-full h-full border border-gray-200 rounded-lg overflow-hidden">
      <Editor
        height={height}
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
          fontSize: 14,
        }}
      />
    </div>
  );
};
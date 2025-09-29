import React, { useRef, useCallback, useEffect } from 'react';
import Editor, { OnChange, OnMount } from '@monaco-editor/react';
import * as monaco from 'monaco-editor';
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

// Generate TypeScript declarations for Tonk globals (minimal, since DOM types come from lib.dom.d.ts)
const generateMonacoTypes = (): string => {
  const packages = buildAvailablePackages();

  let declarations = `
/// <reference lib="dom" />
/// <reference lib="dom.iterable" />
/// <reference lib="es2020" />

// React namespace with minimal types (DOM types come from lib.dom.d.ts)
declare namespace React {
  type FC<P = {}> = (props: P) => ReactElement | null;
  type FunctionComponent<P = {}> = FC<P>;
  type ComponentType<P = {}> = FC<P> | any;
  type ReactNode = ReactElement | string | number | boolean | null | undefined;
  type ReactElement = any;
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

  interface HTMLAttributes<T = any> {
    className?: string;
    id?: string;
    style?: CSSProperties;
    onClick?: (event: MouseEvent) => void;
    onChange?: (event: ChangeEvent<T>) => void;
    onSubmit?: (event: FormEvent) => void;
    children?: ReactNode;
    [key: string]: any;
  }

  interface ChangeEvent<T = HTMLInputElement> extends Event {
    target: EventTarget & T;
    currentTarget: EventTarget & T;
  }

  interface MouseEvent<T = HTMLElement> extends Event {
    clientX: number;
    clientY: number;
  }

  interface FormEvent<T = HTMLFormElement> extends Event {
    target: EventTarget & T;
  }
}

declare const React: {
  createElement: (type: any, props?: any, ...children: any[]) => React.ReactElement;
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

// Export all React hooks as globals for Tonk
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
  const isUserEditingRef = useRef<boolean>(false);
  const lastExternalValueRef = useRef<string>(value);
  const decorationsRef = useRef<string[]>([]);

  // Track when user is actively editing
  useEffect(() => {
    if (editorRef.current) {
      const editor = editorRef.current;

      // Listen for focus/blur to track when user is editing
      const focusDisposable = editor.onDidFocusEditorWidget(() => {
        isUserEditingRef.current = true;
      });

      const blurDisposable = editor.onDidBlurEditorWidget(() => {
        // Small delay before marking as not editing to handle quick re-focuses
        setTimeout(() => {
          isUserEditingRef.current = false;
        }, 100);
      });

      return () => {
        focusDisposable?.dispose();
        blurDisposable?.dispose();
      };
    }
  }, [editorRef.current]);

  // Handle external value changes while preserving cursor position
  useEffect(() => {
    if (editorRef.current && monacoRef.current) {
      const editor = editorRef.current;
      const currentValue = editor.getValue();

      // Only update if value changed externally and user is not actively editing
      if (value !== currentValue && value !== lastExternalValueRef.current && !isUserEditingRef.current) {
        // Save cursor position and selection
        const position = editor.getPosition();
        const selection = editor.getSelection();

        // Update the value
        editor.setValue(value);

        // Restore cursor position and selection if they're still valid
        if (position) {
          const model = editor.getModel();
          const lineCount = model.getLineCount();
          const validPosition = {
            lineNumber: Math.min(position.lineNumber, lineCount),
            column: Math.min(position.column, model.getLineMaxColumn(Math.min(position.lineNumber, lineCount)))
          };
          editor.setPosition(validPosition);

          // Restore selection if it existed
          if (selection && !selection.isEmpty()) {
            const validSelection = {
              startLineNumber: Math.min(selection.startLineNumber, lineCount),
              startColumn: Math.min(selection.startColumn, model.getLineMaxColumn(Math.min(selection.startLineNumber, lineCount))),
              endLineNumber: Math.min(selection.endLineNumber, lineCount),
              endColumn: Math.min(selection.endColumn, model.getLineMaxColumn(Math.min(selection.endLineNumber, lineCount)))
            };
            editor.setSelection(validSelection);
          }
        }

        lastExternalValueRef.current = value;
      }
    }
  }, [value]);

  // Function to check for type errors (defined early so it can be used in callbacks)
  const checkForTypeErrors = useCallback(() => {
    if (!editorRef.current || !monacoRef.current) return;

    const editor = editorRef.current;
    const model = editor.getModel();
    if (!model) return;

    // Get TypeScript worker and check diagnostics
    monacoRef.current.languages.typescript.getTypeScriptWorker().then(worker => {
      worker(model.uri).then(client => {
        Promise.all([
          client.getSemanticDiagnostics(model.uri.toString()),
          client.getSyntacticDiagnostics(model.uri.toString()),
        ]).then(([semanticDiagnostics, syntacticDiagnostics]) => {
          const allDiagnostics = [...semanticDiagnostics, ...syntacticDiagnostics];

          const tsMarkers = allDiagnostics
            .filter(d => {
              // Filter out module/import related errors
              const code = d.code;
              const ignoreCodes = [2792, 2307, 1192, 2686]; // Removed 2304 from here

              // Special handling for "Cannot find name" errors (2304)
              if (code === 2304 && d.messageText) {
                const message = typeof d.messageText === 'string' ? d.messageText : d.messageText.messageText;
                // Only ignore if it's about imports or modules
                if (message && (
                  message.includes('import') ||
                  message.includes('require') ||
                  message.includes('module') ||
                  message.includes('Cannot find namespace')
                )) {
                  return false;
                }
                // Keep the error if it's about undefined variables
                return true;
              }

              return !ignoreCodes.includes(code);
            })
            .map(d => {
              const start = d.start || 0;
              const length = d.length || 1;
              const startPos = model.getPositionAt(start);
              const endPos = model.getPositionAt(start + length);

              return {
                severity: d.category === 1 ? monacoRef.current.MarkerSeverity.Error :
                         d.category === 0 ? monacoRef.current.MarkerSeverity.Warning :
                         monacoRef.current.MarkerSeverity.Info,
                startLineNumber: startPos.lineNumber,
                startColumn: startPos.column,
                endLineNumber: endPos.lineNumber,
                endColumn: endPos.column,
                message: typeof d.messageText === 'string' ? d.messageText :
                        (d.messageText?.messageText || 'Type error'),
                source: 'TypeScript',
                code: d.code?.toString()
              };
            });

          // Set TypeScript markers
          monacoRef.current.editor.setModelMarkers(model, 'typescript', tsMarkers);
        }).catch(err => {
          console.error('Error getting TypeScript diagnostics:', err);
        });
      }).catch(err => {
        console.error('Error getting TypeScript worker:', err);
      });
    }).catch(err => {
      console.error('Error accessing TypeScript language features:', err);
    });
  }, []);

  const handleEditorDidMount: OnMount = useCallback((editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;
    lastExternalValueRef.current = value;

    // Add save keyboard shortcut
    if (onSave) {
      editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
        onSave();
      });
    }

    // Listen for content changes and re-check types
    const model = editor.getModel();
    if (model) {
      model.onDidChangeContent(() => {
        // Debounce type checking on content change
        setTimeout(() => {
          checkForTypeErrors();
        }, 300);
      });
    }

    // Configure TypeScript compiler options for strict type checking
    const compilerOptions = {
      target: monaco.languages.typescript.ScriptTarget.ES2020,
      module: monaco.languages.typescript.ModuleKind.ESNext,
      jsx: monaco.languages.typescript.JsxEmit.React,
      jsxFactory: 'React.createElement',
      jsxFragmentFactory: 'React.Fragment',
      esModuleInterop: true,
      skipLibCheck: true,
      noEmit: true,
      allowSyntheticDefaultImports: true,
      strict: true,
      noImplicitAny: true,
      strictNullChecks: true,
      strictFunctionTypes: true,
      strictBindCallApply: true,
      noImplicitThis: true,
      alwaysStrict: true,
      noUnusedLocals: false, // Don't warn about unused locals in Tonk
      noUnusedParameters: false, // Don't warn about unused parameters
      noImplicitReturns: true,
      noFallthroughCasesInSwitch: true,
      allowJs: true,
      checkJs: false,
      moduleResolution: monaco.languages.typescript.ModuleResolutionKind.NodeJs,
    };

    // Apply to both TypeScript and JavaScript
    monaco.languages.typescript.typescriptDefaults.setCompilerOptions(compilerOptions);
    monaco.languages.typescript.javascriptDefaults.setCompilerOptions(compilerOptions);

    // Configure diagnostics options - enable all checks
    const diagnosticsOptions = {
      noSemanticValidation: false,  // Enable semantic validation (type checking)
      noSyntaxValidation: false,     // Enable syntax validation
      noSuggestionDiagnostics: false, // Enable suggestions
      diagnosticCodesToIgnore: [
        2792, // Cannot find module - Tonk handles imports differently
        2307, // Cannot find module
        1192, // Module has no default export
        2686, // Refers to UMD global
        2304, // Cannot find name (only for imports)
      ]
    };

    monaco.languages.typescript.typescriptDefaults.setDiagnosticsOptions(diagnosticsOptions);
    monaco.languages.typescript.javascriptDefaults.setDiagnosticsOptions(diagnosticsOptions);

    // Set eager model sync for better performance with dynamic models
    monaco.languages.typescript.typescriptDefaults.setEagerModelSync(true);
    monaco.languages.typescript.javascriptDefaults.setEagerModelSync(true);

    // Add Tonk global type declarations
    const tonkTypes = generateMonacoTypes();

    // Clear existing libs and add new ones
    monaco.languages.typescript.typescriptDefaults.setExtraLibs([]);
    monaco.languages.typescript.javascriptDefaults.setExtraLibs([]);

    monaco.languages.typescript.typescriptDefaults.addExtraLib(
      tonkTypes,
      'ts:tonk-globals.d.ts'
    );
    monaco.languages.typescript.javascriptDefaults.addExtraLib(
      tonkTypes,
      'ts:tonk-globals.d.ts'
    );

    // Force initial validation
    if (model) {
      // Trigger validation by making a small change and reverting it
      const originalValue = model.getValue();
      model.setValue(originalValue + ' ');
      model.setValue(originalValue);
    }

    // Set up error checking
    checkForTypeErrors();
  }, [onSave, value, checkForTypeErrors]);

  // Update error markers and check types when content changes
  useEffect(() => {
    if (editorRef.current && monacoRef.current) {
      const model = editorRef.current.getModel();
      if (model) {
        // Set Tonk validation errors
        const tonkMarkers = errors.map(err => ({
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
          source: 'Tonk Validation',
        }));
        monacoRef.current.editor.setModelMarkers(model, 'tonk', tonkMarkers);

        // Always check for TypeScript errors when value changes
        // Use multiple timeouts to ensure we catch the errors
        checkForTypeErrors(); // Immediate check
        setTimeout(() => checkForTypeErrors(), 100); // Quick follow-up
        setTimeout(() => checkForTypeErrors(), 500); // Delayed check
        setTimeout(() => checkForTypeErrors(), 1000); // Final check
      }
    }
  }, [errors, value, checkForTypeErrors]);

  // Update TypeScript declarations when components or stores change
  useEffect(() => {
    if (monacoRef.current) {
      const tonkTypes = generateMonacoTypes();

      // Clear and re-add libs
      monacoRef.current.languages.typescript.typescriptDefaults.setExtraLibs([]);
      monacoRef.current.languages.typescript.javascriptDefaults.setExtraLibs([]);

      monacoRef.current.languages.typescript.typescriptDefaults.addExtraLib(
        tonkTypes,
        'ts:tonk-globals.d.ts'
      );
      monacoRef.current.languages.typescript.javascriptDefaults.addExtraLib(
        tonkTypes,
        'ts:tonk-globals.d.ts'
      );

      // Re-check for errors
      checkForTypeErrors();
    }

    // Subscribe to registry changes
    const unsubComponents = componentRegistry.onContextUpdate(() => {
      if (monacoRef.current) {
        const tonkTypes = generateMonacoTypes();
        monacoRef.current.languages.typescript.typescriptDefaults.setExtraLibs([]);
        monacoRef.current.languages.typescript.typescriptDefaults.addExtraLib(
          tonkTypes,
          'ts:tonk-globals.d.ts'
        );
        checkForTypeErrors();
      }
    });

    const unsubStores = storeRegistry.onContextUpdate(() => {
      if (monacoRef.current) {
        const tonkTypes = generateMonacoTypes();
        monacoRef.current.languages.typescript.typescriptDefaults.setExtraLibs([]);
        monacoRef.current.languages.typescript.typescriptDefaults.addExtraLib(
          tonkTypes,
          'ts:tonk-globals.d.ts'
        );
        checkForTypeErrors();
      }
    });

    return () => {
      unsubComponents();
      unsubStores();
    };
  }, [checkForTypeErrors]);

  const handleChange: OnChange = useCallback((value) => {
    onChange(value || '');
  }, [onChange]);

  // Configure Monaco before mount
  const handleEditorWillMount = useCallback((monaco: any) => {
    // Pre-configure TypeScript defaults before editor mounts
    monaco.languages.typescript.typescriptDefaults.setCompilerOptions({
      target: monaco.languages.typescript.ScriptTarget.ES2020,
      module: monaco.languages.typescript.ModuleKind.ESNext,
      jsx: monaco.languages.typescript.JsxEmit.React,
      strict: true,
      noImplicitAny: true,
      strictNullChecks: true,
      lib: ['es2020', 'dom'],
    });

    monaco.languages.typescript.typescriptDefaults.setDiagnosticsOptions({
      noSemanticValidation: false,
      noSyntaxValidation: false,
      noSuggestionDiagnostics: false,
    });

    monaco.languages.typescript.typescriptDefaults.setEagerModelSync(true);
  }, []);

  return (
    <div className="w-full h-full overflow-hidden">
      <Editor
        height={height}
        language={language}
        value={value}
        onChange={handleChange}
        onMount={handleEditorDidMount}
        beforeMount={handleEditorWillMount}
        theme="vs-dark"
        path={filePath || 'file:///tonk-component.tsx'}
        className="border-0"
        options={{
          readOnly,
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          wordWrap: 'on',
          automaticLayout: true,
          fontSize: 14,
          quickSuggestions: {
            other: true,
            comments: true,
            strings: true
          },
          parameterHints: {
            enabled: true
          },
          suggestOnTriggerCharacters: true,
          acceptSuggestionOnEnter: 'on',
          tabCompletion: 'on',
          wordBasedSuggestions: "allDocuments",
        }}
      />
    </div>
  );
};
import React, { useState, useEffect, useRef } from 'react';
import { Play, Code, AlertCircle } from 'lucide-react';

interface CompilationResult {
  success: boolean;
  component?: any;
  error?: string;
}

interface CompilerProps {
  initialCode?: string;
  height?: string;
  onCompiled?: (result: any) => void;
}

/**
 * Simplified TypeScript compiler that assumes all dependencies are globally available
 */
export const Compiler: React.FC<CompilerProps> = ({
  initialCode = `export default function MyComponent() {
  const [count, setCount] = useState(0);
  
  return (
    <div>
      <h2>Counter: {count}</h2>
      <button onClick={() => setCount(count + 1)}>
        Increment
      </button>
    </div>
  );
}`,
  height = '400px',
  onCompiled,
}) => {
  const [code, setCode] = useState(initialCode);
  const [isCompiling, setIsCompiling] = useState(false);
  const [result, setResult] = useState<CompilationResult | null>(null);
  const sandboxRef = useRef<HTMLDivElement>(null);

  // Available packages (no imports needed by user)
  const availablePackages = {
    React: (window as any).React,
    useState: (window as any).React?.useState,
    useEffect: (window as any).React?.useEffect,
    useCallback: (window as any).React?.useCallback,
    useMemo: (window as any).React?.useMemo,
    useRef: (window as any).React?.useRef,
    useReducer: (window as any).React?.useReducer,
    useContext: (window as any).React?.useContext,
    Fragment: (window as any).React?.Fragment,
  };

  const compileAndExecute = async () => {
    setIsCompiling(true);
    setResult(null);

    try {
      // Load TypeScript if not already loaded
      const ts = (window as any).ts;
      if (!ts) {
        throw new Error('TypeScript compiler not loaded');
      }

      // Compile TypeScript to JavaScript
      const compiled = ts.transpileModule(code, {
        compilerOptions: {
          jsx: ts.JsxEmit.React,
          module: ts.ModuleKind.CommonJS,
          target: ts.ScriptTarget.ES2020,
        },
      });

      // Create context with all available packages
      const contextKeys = Object.keys(availablePackages);
      const contextValues = Object.values(availablePackages);

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

      setResult({ success: true, component });

      // Render the component if it's a React component
      if (typeof component === 'function') {
        renderComponent(component);
      }

      onCompiled?.(component);
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : 'Unknown error';
      setResult({ success: false, error: errorMessage });
    } finally {
      setIsCompiling(false);
    }
  };

  const renderComponent = (Component: React.ComponentType) => {
    if (!sandboxRef.current) return;

    const React = (window as any).React;
    const ReactDOM = (window as any).ReactDOM;

    if (!React || !ReactDOM) {
      setResult({
        success: false,
        error: 'React or ReactDOM not available',
      });
      return;
    }

    try {
      // Clear previous content
      if (ReactDOM.unmountComponentAtNode) {
        ReactDOM.unmountComponentAtNode(sandboxRef.current);
      }

      // Create element and render
      const element = React.createElement(Component);

      // Use React 18 API if available, otherwise fall back to React 17
      if (ReactDOM.createRoot) {
        const root = ReactDOM.createRoot(sandboxRef.current);
        root.render(element);
      } else {
        ReactDOM.render(element, sandboxRef.current);
      }
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : 'Render failed';
      setResult({ success: false, error: errorMessage });
    }
  };

  // Load TypeScript on mount
  useEffect(() => {
    const loadTypeScript = async () => {
      if (!(window as any).ts) {
        const script = document.createElement('script');
        script.src = '/typescript.js';
        script.onload = () => {
          console.log('TypeScript loaded');
        };
        document.head.appendChild(script);
      }
    };
    loadTypeScript();
  }, []);

  return (
    <div className="w-full bg-white rounded-lg border shadow-sm">
      {/* Header */}
      <div className="flex items-center justify-between p-4 border-b bg-gray-50">
        <div className="flex items-center gap-2">
          <Code className="w-5 h-5 text-blue-500" />
          <h3 className="font-semibold text-gray-800">TypeScript Compiler</h3>
          {isCompiling && (
            <div className="flex items-center gap-1 text-blue-600">
              <div className="w-4 h-4 border-2 border-blue-600 border-t-transparent rounded-full animate-spin" />
              <span className="text-sm">Compiling...</span>
            </div>
          )}
        </div>

        <button
          onClick={compileAndExecute}
          disabled={isCompiling}
          className="flex items-center gap-1 px-3 py-1 text-sm bg-blue-500 text-white rounded hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <Play className="w-4 h-4" />
          Compile & Run
        </button>
      </div>

      {/* Code Editor */}
      <div className="relative">
        <textarea
          value={code}
          onChange={e => setCode(e.target.value)}
          placeholder="Write TypeScript code here... No imports needed!"
          className="w-full p-4 font-mono text-sm border-0 resize-none focus:outline-none bg-gray-50"
          style={{ height, minHeight: '200px' }}
          spellCheck={false}
        />
      </div>

      {/* Error Display */}
      {result && !result.success && (
        <div className="p-4 bg-red-50 border-t border-red-200">
          <div className="flex items-start gap-2">
            <AlertCircle className="w-5 h-5 text-red-600 mt-0.5" />
            <div className="flex-1">
              <h4 className="font-medium text-red-800 mb-1">Error</h4>
              <div className="text-sm text-red-700 font-mono">
                {result.error}
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Sandbox Render Area */}
      <div className="border-t bg-gray-50">
        <div className="px-4 py-2 border-b bg-gray-100">
          <h4 className="text-sm font-medium text-gray-700">Output</h4>
        </div>
        <div ref={sandboxRef} className="p-4" style={{ minHeight: '200px' }}>
          {/* Components render here */}
        </div>
      </div>
    </div>
  );
};

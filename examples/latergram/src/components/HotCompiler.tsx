import React, { useState, useEffect, useRef, useCallback } from 'react';
import { Code, AlertCircle, Circle } from 'lucide-react';
import { componentRegistry } from './ComponentRegistry';
import { useDebounce } from './hooks/useDebounce';

interface CompilationResult {
  success: boolean;
  component?: any;
  error?: string;
  preservedState?: boolean;
}

interface HotCompilerProps {
  initialCode?: string;
  height?: string;
  onCompiled?: (result: any) => void;
  autoCompile?: boolean;
  debounceDelay?: number;
}

type CompilationStatus = 'idle' | 'compiling' | 'success' | 'error';

export const HotCompiler: React.FC<HotCompilerProps> = ({
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
  debounceDelay = 600,
}) => {
  const [code, setCode] = useState(initialCode);
  const [status, setStatus] = useState<CompilationStatus>('idle');
  const [result, setResult] = useState<CompilationResult | null>(null);
  const [compilationTime, setCompilationTime] = useState<number | null>(null);

  const sandboxRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<any>(null);
  const componentIdRef = useRef<string>('main-component');
  const lastCompiledCodeRef = useRef<string>('');

  const debouncedCode = useDebounce(code, debounceDelay);

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

  const compileAndExecute = useCallback(
    async (forceRecompile = false) => {
      if (!forceRecompile && code === lastCompiledCodeRef.current) {
        return;
      }

      const startTime = performance.now();
      setStatus('compiling');
      setResult(null);

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

        const componentId = componentIdRef.current;
        const existingComponent = componentRegistry.getComponent(componentId);

        let preservedState = false;
        if (existingComponent) {
          preservedState = componentRegistry.update(componentId, component);
        } else {
          componentRegistry.register(componentId, component);
        }

        const endTime = performance.now();
        setCompilationTime(Math.round(endTime - startTime));

        setStatus('success');
        setResult({ success: true, component, preservedState });
        lastCompiledCodeRef.current = code;

        if (typeof component === 'function') {
          renderComponent(componentId);
        }

        onCompiled?.(component);
      } catch (error) {
        const errorMessage =
          error instanceof Error ? error.message : 'Unknown error';
        setStatus('error');
        setResult({ success: false, error: errorMessage });
      }
    },
    [code, availablePackages, onCompiled]
  );

  const renderComponent = useCallback((componentId: string) => {
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
      const proxiedComponent = componentRegistry.getComponent(componentId);
      if (!proxiedComponent) {
        throw new Error('Component not registered');
      }

      const element = React.createElement(proxiedComponent.proxy);

      if (!rootRef.current) {
        if (ReactDOM.createRoot) {
          rootRef.current = ReactDOM.createRoot(sandboxRef.current);
        } else {
          rootRef.current = {
            render: (el: any) => ReactDOM.render(el, sandboxRef.current),
          };
        }
      }

      rootRef.current.render(element);
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : 'Render failed';
      setResult({ success: false, error: errorMessage });
    }
  }, []);

  useEffect(() => {
    const loadTypeScript = async () => {
      if (!(window as any).ts) {
        const script = document.createElement('script');
        script.src = '/typescript.js';
        script.onload = () => {
          console.log('TypeScript loaded');
          compileAndExecute(true);
        };
        document.head.appendChild(script);
      } else {
        compileAndExecute(true);
      }
    };
    loadTypeScript();
  }, []);

  useEffect(() => {
    if (debouncedCode) {
      compileAndExecute();
    }
  }, [debouncedCode, compileAndExecute]);

  useEffect(() => {
    return () => {
      if (rootRef.current && (window as any).ReactDOM?.unmountComponentAtNode) {
        (window as any).ReactDOM.unmountComponentAtNode(sandboxRef.current);
      }
    };
  }, []);

  const getStatusColor = () => {
    switch (status) {
      case 'compiling':
        return 'text-blue-500';
      case 'success':
        return 'text-green-500';
      case 'error':
        return 'text-red-500';
      default:
        return 'text-gray-400';
    }
  };

  const getStatusIcon = () => {
    switch (status) {
      case 'compiling':
        return (
          <div className="w-4 h-4 border-2 border-blue-500 border-t-transparent rounded-full animate-spin" />
        );
      case 'success':
        return <Circle className="w-4 h-4 fill-green-500 text-green-500" />;
      case 'error':
        return <Circle className="w-4 h-4 fill-red-500 text-red-500" />;
      default:
        return <Circle className="w-4 h-4 text-gray-400" />;
    }
  };

  return (
    <div className="w-full bg-white rounded-lg border shadow-sm">
      <div className="flex items-center justify-between p-4 border-b bg-gray-50">
        <div className="flex items-center gap-3">
          <Code className="w-5 h-5 text-blue-500" />
          <h3 className="font-semibold text-gray-800">TypeScript Compiler</h3>

          <div className="flex items-center gap-2">
            {getStatusIcon()}
            <span className={`text-sm ${getStatusColor()}`}>
              {status === 'compiling'
                ? 'Compiling...'
                : status === 'success'
                  ? `Compiled${result?.preservedState ? ' (state preserved)' : ''} in ${compilationTime}ms`
                  : status === 'error'
                    ? 'Error'
                    : 'Ready'}
            </span>
          </div>
        </div>
      </div>

      <div className="relative">
        <textarea
          value={code}
          onChange={e => setCode(e.target.value)}
          placeholder="Write TypeScript code here... No imports needed!"
          className="w-full p-4 font-mono text-sm border-0 resize-none focus:outline-none bg-gray-50"
          style={{ height, minHeight: '200px' }}
          spellCheck={false}
        />

        {status === 'compiling' && (
          <div className="absolute top-2 right-2 px-2 py-1 bg-blue-100 text-blue-700 text-xs rounded">
            Compiling...
          </div>
        )}
      </div>

      {result && !result.success && (
        <div className="p-4 bg-red-50 border-t border-red-200">
          <div className="flex items-start gap-2">
            <AlertCircle className="w-5 h-5 text-red-600 mt-0.5" />
            <div className="flex-1">
              <h4 className="font-medium text-red-800 mb-1">
                Compilation Error
              </h4>
              <div className="text-sm text-red-700 font-mono whitespace-pre-wrap">
                {result.error}
              </div>
            </div>
          </div>
        </div>
      )}

      <div className="border-t bg-gray-50">
        <div className="px-4 py-2 border-b bg-gray-100 flex items-center justify-between">
          <h4 className="text-sm font-medium text-gray-700">Output</h4>
        </div>
        <div
          ref={sandboxRef}
          className="p-4"
          style={{ minHeight: '200px' }}
        ></div>
      </div>
    </div>
  );
};

import { buildAvailablePackages } from '../../components/contextBuilder';

export interface CompilationResult {
  success: boolean;
  output?: any;
  error?: string;
}

export interface CompilerConfig {
  moduleType?: 'None' | 'CommonJS';
  outputFormat?: 'default' | 'module';
  excludeStoreId?: string;
  wrapWithChakra?: boolean;
}

export const compileTSCode = async (
  code: string,
  config: CompilerConfig = {}
): Promise<CompilationResult> => {
  try {
    const ts = (window as any).ts;
    if (!ts) {
      throw new Error('TypeScript compiler not loaded');
    }

    const { 
      moduleType = 'None', 
      outputFormat = 'default',
      wrapWithChakra = false 
    } = config;

    const compiled = ts.transpileModule(code, {
      compilerOptions: {
        jsx: ts.JsxEmit.React,
        module:
          moduleType === 'CommonJS'
            ? ts.ModuleKind.CommonJS
            : ts.ModuleKind.None,
        target: ts.ScriptTarget.ES2020,
      },
    });

    const freshPackages = buildAvailablePackages(config.excludeStoreId);
    const contextKeys = Object.keys(freshPackages);
    const contextValues = Object.values(freshPackages);

    let moduleFactory: Function;
    if (outputFormat === 'module') {
      moduleFactory = new Function(
        ...contextKeys,
        `
        const exports = {};
        const module = { exports };
        ${compiled.outputText}
        return module.exports.default || module.exports;
        `
      );
    } else {
      moduleFactory = new Function(...contextKeys, compiled.outputText);
    }

    let result = moduleFactory(...contextValues);

    if (wrapWithChakra && result) {
      const React = (window as any).React;
      const ChakraProvider = (window as any).ChakraUI?.ChakraProvider;
      const defaultSystem = (window as any).defaultSystem;

      if (React && ChakraProvider && defaultSystem) {
        const OriginalComponent = result;
        result = (props: any) => {
          return React.createElement(
            ChakraProvider,
            { value: defaultSystem },
            React.createElement(OriginalComponent, props)
          );
        };
        result.displayName = `ChakraWrapped(${OriginalComponent.displayName || OriginalComponent.name || 'Component'})`;
      }
    }

    return {
      success: true,
      output: result,
    };
  } catch (error) {
    return {
      success: false,
      error: error instanceof Error ? error.message : 'Compilation failed',
    };
  }
};

export const ensureTypeScriptLoaded = (): Promise<void> => {
  return new Promise(resolve => {
    if ((window as any).ts) {
      resolve();
    } else {
      const script = document.createElement('script');
      script.src = '/typescript.js';
      script.onload = () => resolve();
      document.head.appendChild(script);
    }
  });
};

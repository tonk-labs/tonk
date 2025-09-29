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

    const { moduleType = 'None', outputFormat = 'default' } = config;

    // Compile the TypeScript code
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

    // Get fresh context, excluding the current store if specified
    const freshPackages = buildAvailablePackages(config.excludeStoreId);
    const contextKeys = Object.keys(freshPackages);
    const contextValues = Object.values(freshPackages);

    // Handle different output formats
    let moduleFactory: Function;
    if (outputFormat === 'module') {
      // For components that use module.exports
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
      // For stores that use direct return
      moduleFactory = new Function(...contextKeys, compiled.outputText);
    }

    const result = moduleFactory(...contextValues);

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

import * as ts from 'typescript';

export interface TypeCheckResult {
  valid: boolean;
  diagnostics: TypeCheckDiagnostic[];
  errorCount: number;
  warningCount: number;
}

export interface TypeCheckDiagnostic {
  file: string;
  line: number;
  column: number;
  message: string;
  category: 'error' | 'warning' | 'info';
  code: number;
}

export class TypeScriptValidator {
  private compilerOptions: ts.CompilerOptions = {
    target: ts.ScriptTarget.ES2020,
    module: ts.ModuleKind.ESNext,
    lib: ['ES2020', 'DOM', 'DOM.Iterable'],
    jsx: ts.JsxEmit.ReactJSX,
    jsxImportSource: 'react',  // Specify where to import jsx from
    strict: false,  // Less strict for Tonk components
    esModuleInterop: true,
    skipLibCheck: true,
    forceConsistentCasingInFileNames: true,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    allowImportingTsExtensions: true,
    noEmit: true,
    isolatedModules: false,
    noImplicitAny: false,
    allowSyntheticDefaultImports: true,
    noLib: false,  // Make sure we use the standard library
  };

  // Ambient declarations for Tonk component context (injected dependencies)
  private tonkAmbientDeclarations = `
    /// <reference lib="es2020" />
    /// <reference lib="dom" />
    import * as React from 'react';

    declare global {
      const React: typeof import('react');
      const useState: typeof React.useState;
      const useEffect: typeof React.useEffect;
      const useCallback: typeof React.useCallback;
      const useMemo: typeof React.useMemo;
      const useRef: typeof React.useRef;
      const useReducer: typeof React.useReducer;
      const useContext: typeof React.useContext;
      const Fragment: typeof React.Fragment;

      // Zustand for stores
      const create: any;
      const sync: any;

      // Allow any use* hook
      function use[A-Z][a-zA-Z]*(...args: any[]): any;

      // Component types
      type FC<P = {}> = React.FC<P>;
      type ReactNode = React.ReactNode;
      type ReactElement = React.ReactElement;
      type MouseEvent = React.MouseEvent;
      type ChangeEvent = React.ChangeEvent;
      type FormEvent = React.FormEvent;
      type KeyboardEvent = React.KeyboardEvent;
      type CSSProperties = React.CSSProperties;

      namespace JSX {
        interface IntrinsicElements {
          [elemName: string]: any;
        }
        interface Element extends React.ReactElement<any, any> { }
        interface ElementClass extends React.Component<any> {
          render(): React.ReactNode;
        }
        interface ElementAttributesProperty { props: {}; }
        interface ElementChildrenAttribute { children: {}; }
        interface IntrinsicAttributes extends React.Attributes { }
        interface IntrinsicClassAttributes<T> extends React.ClassAttributes<T> { }
      }
    }

    export {};
  `;

  private fileVersions = new Map<string, number>();
  private sourceFiles = new Map<string, ts.SourceFile>();

  /**
   * Check if this is a Tonk component (no imports, uses React without importing)
   */
  private isTonkComponent(content: string): boolean {
    // Check if there are no React imports but uses React/hooks/JSX
    const hasReactImports = /^import\s+.*from\s+['"]react['"]/m.test(content);
    const usesReact = /\b(React\.|useState|useEffect|useCallback|useMemo|useRef|<[A-Z]|\bFC\b)/g.test(content);
    const hasExportDefault = /export\s+default/g.test(content);

    // It's a Tonk component if it uses React features without React imports
    // (it may have other imports, just not React ones)
    return !hasReactImports && usesReact && hasExportDefault;
  }

  /**
   * Prepare content for validation (add ambient declarations for Tonk components)
   */
  private prepareContent(filePath: string, content: string): string {
    // Check if this is a component file in the Tonk environment
    const isComponentFile = filePath.includes('/components/') || filePath.includes('/views/');

    if (isComponentFile && this.isTonkComponent(content)) {
      // Prepend ambient declarations for Tonk components
      return this.tonkAmbientDeclarations + '\n' + content;
    }

    return content;
  }

  /**
   * Validate a single file with TypeScript compiler
   */
  async validateFile(
    filePath: string,
    content: string,
    additionalFiles?: Map<string, string>
  ): Promise<TypeCheckResult> {
    // Prepare content (add ambient declarations if needed)
    const preparedContent = this.prepareContent(filePath, content);

    // Create a custom compiler host
    const host = this.createCompilerHost(filePath, preparedContent, additionalFiles);

    // Create the program
    const program = ts.createProgram(
      [filePath],
      this.compilerOptions,
      host
    );

    // Get all diagnostics
    const syntacticDiagnostics = program.getSyntacticDiagnostics();
    const semanticDiagnostics = program.getSemanticDiagnostics();
    const allDiagnostics = [...syntacticDiagnostics, ...semanticDiagnostics];

    // Filter out diagnostics that are expected in Tonk components
    const filteredDiagnostics = this.filterTonkDiagnostics(allDiagnostics, content);

    // Convert diagnostics to our format
    const diagnostics = this.convertDiagnostics(filteredDiagnostics);

    return {
      valid: diagnostics.filter(d => d.category === 'error').length === 0,
      diagnostics,
      errorCount: diagnostics.filter(d => d.category === 'error').length,
      warningCount: diagnostics.filter(d => d.category === 'warning').length,
    };
  }

  /**
   * Filter out diagnostics that are expected in Tonk components
   */
  private filterTonkDiagnostics(
    diagnostics: readonly ts.Diagnostic[],
    originalContent: string
  ): ts.Diagnostic[] {
    const isTonk = this.isTonkComponent(originalContent);

    if (!isTonk) {
      return [...diagnostics];
    }

    return diagnostics.filter(diagnostic => {
      const message = ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n');

      // Filter out "cannot find name" errors for injected dependencies
      if (message.includes('Cannot find name') && (
        message.includes('use') || // Any hook
        message.includes('React') ||
        message.includes('FC') ||
        message.includes('create')
      )) {
        return false;
      }

      // Filter out import-related errors for Tonk components
      if (message.includes('Cannot find module') ||
          message.includes('Could not find a declaration file')) {
        return false;
      }

      return true;
    });
  }

  /**
   * Validate multiple files as a project
   */
  async validateProject(
    files: Map<string, string>
  ): Promise<Map<string, TypeCheckResult>> {
    const results = new Map<string, TypeCheckResult>();
    const filePaths = Array.from(files.keys());

    // Prepare all files (add ambient declarations if needed)
    const preparedFiles = new Map<string, string>();
    files.forEach((content, path) => {
      preparedFiles.set(path, this.prepareContent(path, content));
    });

    // Create a custom compiler host with all prepared files
    const host = this.createProjectCompilerHost(preparedFiles);

    // Create the program with all files
    const program = ts.createProgram(
      filePaths,
      this.compilerOptions,
      host
    );

    // Get diagnostics for each file
    for (const filePath of filePaths) {
      const sourceFile = program.getSourceFile(filePath);
      if (!sourceFile) continue;

      const syntacticDiagnostics = program.getSyntacticDiagnostics(sourceFile);
      const semanticDiagnostics = program.getSemanticDiagnostics(sourceFile);
      const allDiagnostics = [...syntacticDiagnostics, ...semanticDiagnostics];

      // Filter diagnostics for Tonk components
      const originalContent = files.get(filePath) || '';
      const filteredDiagnostics = this.filterTonkDiagnostics(allDiagnostics, originalContent);

      const diagnostics = this.convertDiagnostics(filteredDiagnostics);

      results.set(filePath, {
        valid: diagnostics.filter(d => d.category === 'error').length === 0,
        diagnostics,
        errorCount: diagnostics.filter(d => d.category === 'error').length,
        warningCount: diagnostics.filter(d => d.category === 'warning').length,
      });
    }

    return results;
  }

  private createCompilerHost(
    mainFile: string,
    mainContent: string,
    additionalFiles?: Map<string, string>
  ): ts.CompilerHost {
    const files = new Map<string, string>();
    files.set(mainFile, mainContent);

    if (additionalFiles) {
      additionalFiles.forEach((content, path) => {
        files.set(path, content);
      });
    }

    // Add React type stubs
    const reactTypesStub = `
      export = React;
      export as namespace React;

      declare namespace React {
        type FC<P = {}> = FunctionComponent<P>;
        type FunctionComponent<P = {}> = (props: P) => ReactElement | null;
        type ReactNode = ReactElement | string | number | boolean | null | undefined;
        interface ReactElement<P = any> {
          type: any;
          props: P;
          key: any;
        }
        function useState<T>(initial: T): [T, (value: T) => void];
        function useEffect(effect: () => void, deps?: any[]): void;
        function useCallback<T extends Function>(callback: T, deps: any[]): T;
        function useMemo<T>(factory: () => T, deps: any[]): T;
        function useRef<T>(initial?: T): { current: T };
        function useReducer<S, A>(reducer: (state: S, action: A) => S, initial: S): [S, (action: A) => void];
        function useContext<T>(context: any): T;
        const Fragment: any;
        interface Component<P = {}> {
          render(): ReactNode;
        }
        interface ClassAttributes<T> {}
        interface Attributes {}
        interface MouseEvent {}
        interface ChangeEvent {}
        interface FormEvent {}
        interface KeyboardEvent {}
        interface CSSProperties {
          [key: string]: any;
        }
      }
    `;

    // JSX runtime stub for ReactJSX mode
    const jsxRuntimeStub = `
      import * as React from 'react';
      export function jsx(type: any, props: any, key?: any): any;
      export function jsxs(type: any, props: any, key?: any): any;
      export function jsxDEV(type: any, props: any, key: any, isStatic: boolean, source: any, self: any): any;
      export const Fragment = React.Fragment;
    `;

    return {
      getSourceFile: (fileName, languageVersion) => {
        // Normalize the filename for comparison
        const normalizedFileName = fileName.toLowerCase();

        // Provide JSX runtime for the new JSX transform - be more flexible with path matching
        if (normalizedFileName.includes('jsx-runtime') || normalizedFileName.includes('jsx-dev-runtime')) {
          return ts.createSourceFile(fileName, jsxRuntimeStub, languageVersion, true);
        }

        // Provide React types
        if (normalizedFileName.includes('/react/index') || normalizedFileName.endsWith('/react.d.ts') || normalizedFileName === 'react') {
          return ts.createSourceFile(fileName, reactTypesStub, languageVersion, true);
        }

        // Check our files first
        if (files.has(fileName)) {
          const content = files.get(fileName)!;
          return ts.createSourceFile(fileName, content, languageVersion, true);
        }

        // Provide stubs for node_modules TypeScript is looking for
        if (fileName.includes('node_modules')) {
          if (fileName.includes('react')) {
            return ts.createSourceFile(fileName, reactTypesStub, languageVersion, true);
          }
          // Return empty stub for other node_modules
          return ts.createSourceFile(fileName, '', languageVersion, true);
        }

        // Return undefined for system files (they'll be handled by default lib)
        if (fileName.includes('lib.')) {
          return undefined;
        }

        // Try to create a stub for imports we don't have
        if (fileName.endsWith('.ts') || fileName.endsWith('.tsx')) {
          return ts.createSourceFile(fileName, '', languageVersion, true);
        }

        return undefined;
      },
      writeFile: () => {}, // No-op since we're not emitting
      getCurrentDirectory: () => '/',
      getDirectories: () => [],
      fileExists: (fileName) => {
        // Say yes for jsx-runtime so TypeScript knows it's available
        if (fileName.toLowerCase().includes('jsx-runtime')) return true;
        if (fileName.toLowerCase().includes('jsx-dev-runtime')) return true;
        if (fileName.toLowerCase().includes('/react/') || fileName.toLowerCase().endsWith('/react.d.ts')) return true;
        if (fileName.includes('node_modules') && fileName.includes('react')) return true;
        return files.has(fileName);
      },
      readFile: (fileName) => {
        if (fileName.toLowerCase().includes('jsx-runtime')) return jsxRuntimeStub;
        if (fileName.toLowerCase() === 'react' || fileName.toLowerCase().endsWith('/react')) return reactTypesStub;
        return files.get(fileName);
      },
      getCanonicalFileName: (fileName) => fileName,
      useCaseSensitiveFileNames: () => true,
      getNewLine: () => '\n',
      getDefaultLibFileName: (options) => ts.getDefaultLibFileName(options),
      getDefaultLibLocation: () => ts.getDefaultLibFilePath(this.compilerOptions),
      resolveModuleNames: (moduleNames: string[], containingFile: string) => {
        return moduleNames.map(moduleName => {
          // Handle jsx-runtime resolution - TypeScript needs these for ReactJSX mode
          if (moduleName === 'react/jsx-runtime' || moduleName === 'react/jsx-dev-runtime') {
            // Return a valid resolved module that TypeScript will accept
            return {
              resolvedFileName: `/node_modules/${moduleName}.d.ts`,
              isExternalLibraryImport: true,
              extension: ts.Extension.Dts,
            } as ts.ResolvedModule;
          }
          // Handle React resolution
          if (moduleName === 'react') {
            return {
              resolvedFileName: '/node_modules/react/index.d.ts',
              isExternalLibraryImport: true,
              extension: ts.Extension.Dts,
            } as ts.ResolvedModule;
          }
          // Let other modules be undefined (TypeScript will handle them)
          return undefined;
        });
      },
    };
  }

  private createProjectCompilerHost(
    files: Map<string, string>
  ): ts.CompilerHost {
    // React type stubs (same as in createCompilerHost)
    const reactTypesStub = `
      export = React;
      export as namespace React;
      declare namespace React {
        type FC<P = {}> = FunctionComponent<P>;
        type FunctionComponent<P = {}> = (props: P) => ReactElement | null;
        type ReactNode = ReactElement | string | number | boolean | null | undefined;
        interface ReactElement<P = any> {
          type: any;
          props: P;
          key: any;
        }
        function useState<T>(initial: T): [T, (value: T) => void];
        function useEffect(effect: () => void, deps?: any[]): void;
        function useCallback<T extends Function>(callback: T, deps: any[]): T;
        function useMemo<T>(factory: () => T, deps: any[]): T;
        function useRef<T>(initial?: T): { current: T };
        function useReducer<S, A>(reducer: (state: S, action: A) => S, initial: S): [S, (action: A) => void];
        function useContext<T>(context: any): T;
        const Fragment: any;
        interface Component<P = {}> {
          render(): ReactNode;
        }
        interface ClassAttributes<T> {}
        interface Attributes {}
      }
    `;

    const jsxRuntimeStub = `
      import * as React from 'react';
      export function jsx(type: any, props: any, key?: any): any;
      export function jsxs(type: any, props: any, key?: any): any;
      export function jsxDEV(type: any, props: any, key: any, isStatic: boolean, source: any, self: any): any;
      export const Fragment = React.Fragment;
    `;

    return {
      getSourceFile: (fileName, languageVersion) => {
        // Provide React types
        if (fileName === 'react' || fileName.includes('react/index')) {
          return ts.createSourceFile(fileName, reactTypesStub, languageVersion, true);
        }

        // Provide JSX runtime
        if (fileName.includes('react/jsx-runtime') || fileName.includes('react/jsx-dev-runtime')) {
          return ts.createSourceFile(fileName, jsxRuntimeStub, languageVersion, true);
        }

        if (files.has(fileName)) {
          const content = files.get(fileName)!;
          return ts.createSourceFile(fileName, content, languageVersion, true);
        }

        // Return undefined for system files
        if (fileName.includes('node_modules') || fileName.includes('lib.')) {
          return undefined;
        }

        // Create stub for missing imports
        if (fileName.endsWith('.ts') || fileName.endsWith('.tsx')) {
          return ts.createSourceFile(fileName, '', languageVersion, true);
        }

        return undefined;
      },
      writeFile: () => {},
      getCurrentDirectory: () => '/',
      getDirectories: () => [],
      fileExists: (fileName) => {
        // Say yes for jsx-runtime so TypeScript knows it's available
        if (fileName.toLowerCase().includes('jsx-runtime')) return true;
        if (fileName.toLowerCase().includes('jsx-dev-runtime')) return true;
        if (fileName.toLowerCase().includes('/react/') || fileName.toLowerCase().endsWith('/react.d.ts')) return true;
        if (fileName.includes('node_modules') && fileName.includes('react')) return true;
        return files.has(fileName);
      },
      readFile: (fileName) => {
        if (fileName.toLowerCase().includes('jsx-runtime')) return jsxRuntimeStub;
        if (fileName.toLowerCase() === 'react' || fileName.toLowerCase().endsWith('/react')) return reactTypesStub;
        return files.get(fileName);
      },
      getCanonicalFileName: (fileName) => fileName,
      useCaseSensitiveFileNames: () => true,
      getNewLine: () => '\n',
      getDefaultLibFileName: (options) => ts.getDefaultLibFileName(options),
      getDefaultLibLocation: () => ts.getDefaultLibFilePath(this.compilerOptions),
      resolveModuleNames: (moduleNames: string[], containingFile: string) => {
        return moduleNames.map(moduleName => {
          // Handle jsx-runtime resolution - TypeScript needs these for ReactJSX mode
          if (moduleName === 'react/jsx-runtime' || moduleName === 'react/jsx-dev-runtime') {
            // Return a valid resolved module that TypeScript will accept
            return {
              resolvedFileName: `/node_modules/${moduleName}.d.ts`,
              isExternalLibraryImport: true,
              extension: ts.Extension.Dts,
            } as ts.ResolvedModule;
          }
          // Handle React resolution
          if (moduleName === 'react') {
            return {
              resolvedFileName: '/node_modules/react/index.d.ts',
              isExternalLibraryImport: true,
              extension: ts.Extension.Dts,
            } as ts.ResolvedModule;
          }
          // Let other modules be undefined (TypeScript will handle them)
          return undefined;
        });
      },
    };
  }

  private convertDiagnostics(diagnostics: readonly ts.Diagnostic[]): TypeCheckDiagnostic[] {
    return diagnostics.map(diagnostic => {
      const file = diagnostic.file;
      let line = 1;
      let column = 1;

      if (file && diagnostic.start !== undefined) {
        const { line: l, character } = file.getLineAndCharacterOfPosition(diagnostic.start);
        line = l + 1;
        column = character + 1;
      }

      let category: 'error' | 'warning' | 'info' = 'error';
      switch (diagnostic.category) {
        case ts.DiagnosticCategory.Warning:
          category = 'warning';
          break;
        case ts.DiagnosticCategory.Message:
        case ts.DiagnosticCategory.Suggestion:
          category = 'info';
          break;
      }

      const message = ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n');

      return {
        file: file?.fileName || 'unknown',
        line,
        column,
        message,
        category,
        code: diagnostic.code,
      };
    });
  }

  /**
   * Generate agent-friendly feedback for TypeScript errors
   */
  generateAgentFeedback(result: TypeCheckResult, filePath: string): string {
    if (result.valid) {
      return `✅ TypeScript validation passed for "${filePath}"`;
    }

    let feedback = `❌ TypeScript errors in "${filePath}":\n\n`;

    // Group by error type
    const typeErrors = result.diagnostics.filter(d =>
      d.message.toLowerCase().includes('type') ||
      d.message.includes('TS2')
    );
    const importErrors = result.diagnostics.filter(d =>
      d.message.includes('import') ||
      d.message.includes('Cannot find')
    );
    const otherErrors = result.diagnostics.filter(d =>
      !typeErrors.includes(d) && !importErrors.includes(d)
    );

    if (typeErrors.length > 0) {
      feedback += `**Type Errors:**\n`;
      typeErrors.forEach(error => {
        feedback += `  Line ${error.line}: ${error.message}\n`;
      });
      feedback += '\n';
    }

    if (importErrors.length > 0) {
      feedback += `**Import/Module Errors:**\n`;
      importErrors.forEach(error => {
        feedback += `  Line ${error.line}: ${error.message}\n`;
      });
      feedback += '\n';
    }

    if (otherErrors.length > 0) {
      feedback += `**Other Errors:**\n`;
      otherErrors.forEach(error => {
        feedback += `  Line ${error.line}: ${error.message}\n`;
      });
    }

    return feedback;
  }
}

export const typeScriptValidator = new TypeScriptValidator();
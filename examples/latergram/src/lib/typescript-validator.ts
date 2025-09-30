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
    module: ts.ModuleKind.CommonJS,
    jsx: ts.JsxEmit.React,
    strict: false,
    esModuleInterop: true,
    skipLibCheck: true,
    forceConsistentCasingInFileNames: true,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    allowImportingTsExtensions: true,
    noEmit: true,
    allowSyntheticDefaultImports: true,
  };

  /**
   * Compile and execute TypeScript code with available packages context
   * This matches the pattern used in HotCompiler.tsx for consistent compilation
   */
  compileWithContext(
    code: string,
    buildAvailablePackages: () => Record<string, any>
  ): { success: boolean; component?: any; error?: string } {
    try {
      const compiled = ts.transpileModule(code, {
        compilerOptions: {
          jsx: ts.JsxEmit.React,
          module: ts.ModuleKind.CommonJS,
          target: ts.ScriptTarget.ES2020,
        },
      });

      // Always get fresh context to ensure all available components are included
      const freshPackages = buildAvailablePackages();
      const contextKeys = Object.keys(freshPackages);
      const contextValues = Object.values(freshPackages);

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

      return { success: true, component };
    } catch (error) {
      return {
        success: false,
        error:
          error instanceof Error ? error.message : 'Unknown compilation error',
      };
    }
  }

  /**
   * Validate TypeScript syntax and get diagnostics
   */
  validateSyntax(code: string): TypeCheckResult {
    const diagnostics: TypeCheckDiagnostic[] = [];
    const lines = code.split('\n');

    // Check file length (max 400 lines)
    if (lines.length > 400) {
      diagnostics.push({
        file: 'component.tsx',
        line: 1,
        column: 1,
        message: `Files Max Lines is 400 LOC, break down files into smaller components and use stores for data storage. Current: ${lines.length} lines`,
        category: 'error',
        code: 1000,
      });
    }

    // Check for import statements (not allowed in Tonk components)
    const importRegex = /^import\s+.+from\s+['"].+['"]/gm;
    const importMatches = [...code.matchAll(importRegex)];

    if (importMatches.length > 0) {
      importMatches.forEach(match => {
        const linesBeforeMatch = code.substring(0, match.index).split('\n');
        const line = linesBeforeMatch.length;
        diagnostics.push({
          file: 'component.tsx',
          line,
          column: 1,
          message: `Import statements are not allowed in Tonk components. All dependencies (React, hooks, components) are injected automatically. Remove: "${match[0]}"`,
          category: 'error',
          code: 1001,
        });
      });
    }

    // Check for require statements (also not allowed)
    const requireRegex = /require\s*\(['"].+['"]\)/g;
    const requireMatches = [...code.matchAll(requireRegex)];

    if (requireMatches.length > 0) {
      requireMatches.forEach(match => {
        const linesBeforeMatch = code.substring(0, match.index).split('\n');
        const line = linesBeforeMatch.length;
        diagnostics.push({
          file: 'component.tsx',
          line,
          column: 1,
          message: `Require statements are not allowed in Tonk components. All dependencies are injected automatically. Remove: "${match[0]}"`,
          category: 'error',
          code: 1002,
        });
      });
    }

    // Parse TypeScript for syntax errors
    const fileName = 'component.tsx';
    const sourceFile = ts.createSourceFile(
      fileName,
      code,
      ts.ScriptTarget.ES2020,
      true
    );

    // Get syntax errors only
    // @ts-expect-error - parseDiagnostics exists at runtime but not in types
    const parseDiagnostics = (sourceFile as any).parseDiagnostics;
    if (parseDiagnostics && parseDiagnostics.length > 0) {
      diagnostics.push(...this.convertDiagnostics(parseDiagnostics));
    }

    return {
      valid: diagnostics.filter(d => d.category === 'error').length === 0,
      diagnostics,
      errorCount: diagnostics.filter(d => d.category === 'error').length,
      warningCount: diagnostics.filter(d => d.category === 'warning').length,
    };
  }

  private convertDiagnostics(
    diagnostics: readonly ts.Diagnostic[]
  ): TypeCheckDiagnostic[] {
    return diagnostics.map(diagnostic => {
      const file = diagnostic.file;
      let line = 1;
      let column = 1;

      if (file && diagnostic.start !== undefined) {
        const { line: l, character } = file.getLineAndCharacterOfPosition(
          diagnostic.start
        );
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

      const message = ts.flattenDiagnosticMessageText(
        diagnostic.messageText,
        '\n'
      );

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

    result.diagnostics.forEach(error => {
      feedback += `  Line ${error.line}: ${error.message}\n`;
    });

    return feedback;
  }
}

export const typeScriptValidator = new TypeScriptValidator();

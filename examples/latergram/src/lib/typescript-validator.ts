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
    lib: ['ES2020', 'DOM'],
    jsx: ts.JsxEmit.ReactJSX,
    strict: true,
    esModuleInterop: true,
    skipLibCheck: true,
    forceConsistentCasingInFileNames: true,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    allowImportingTsExtensions: true,
    noEmit: true,
    isolatedModules: true,
  };

  private fileVersions = new Map<string, number>();
  private sourceFiles = new Map<string, ts.SourceFile>();

  /**
   * Validate a single file with TypeScript compiler
   */
  async validateFile(
    filePath: string,
    content: string,
    additionalFiles?: Map<string, string>
  ): Promise<TypeCheckResult> {
    // Create a custom compiler host
    const host = this.createCompilerHost(filePath, content, additionalFiles);

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

    // Convert diagnostics to our format
    const diagnostics = this.convertDiagnostics(allDiagnostics);

    return {
      valid: diagnostics.filter(d => d.category === 'error').length === 0,
      diagnostics,
      errorCount: diagnostics.filter(d => d.category === 'error').length,
      warningCount: diagnostics.filter(d => d.category === 'warning').length,
    };
  }

  /**
   * Validate multiple files as a project
   */
  async validateProject(
    files: Map<string, string>
  ): Promise<Map<string, TypeCheckResult>> {
    const results = new Map<string, TypeCheckResult>();
    const filePaths = Array.from(files.keys());

    // Create a custom compiler host with all files
    const host = this.createProjectCompilerHost(files);

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

      const diagnostics = this.convertDiagnostics(allDiagnostics);

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

    return {
      getSourceFile: (fileName, languageVersion) => {
        // Check our files first
        if (files.has(fileName)) {
          const content = files.get(fileName)!;
          return ts.createSourceFile(fileName, content, languageVersion, true);
        }

        // Return undefined for system files (they'll be handled by default lib)
        if (fileName.includes('node_modules') || fileName.includes('lib.')) {
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
      fileExists: (fileName) => files.has(fileName),
      readFile: (fileName) => files.get(fileName),
      getCanonicalFileName: (fileName) => fileName,
      useCaseSensitiveFileNames: () => true,
      getNewLine: () => '\n',
      getDefaultLibFileName: (options) => ts.getDefaultLibFileName(options),
    };
  }

  private createProjectCompilerHost(
    files: Map<string, string>
  ): ts.CompilerHost {
    return {
      getSourceFile: (fileName, languageVersion) => {
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
      fileExists: (fileName) => files.has(fileName),
      readFile: (fileName) => files.get(fileName),
      getCanonicalFileName: (fileName) => fileName,
      useCaseSensitiveFileNames: () => true,
      getNewLine: () => '\n',
      getDefaultLibFileName: (options) => ts.getDefaultLibFileName(options),
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
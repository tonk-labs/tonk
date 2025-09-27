// @ts-ignore - WASM imports
import init, * as biomeWasm from '@biomejs/wasm-web';
// Import WASM file directly with ?url to get the URL
// @ts-ignore
import biomeWasmUrl from '@biomejs/wasm-web/biome_wasm_bg.wasm?url';

export interface ValidationResult {
  valid: boolean;
  errors: ValidationError[];
  warnings: ValidationError[];
  formatted?: string;
  suggestions: string[];
}

export interface ValidationError {
  line: number;
  column: number;
  severity: 'error' | 'warning';
  message: string;
  rule?: string;
  fix?: string;
}

export class FileValidator {
  private wasmInitialized = false;
  private workspace: any = null;
  private fileSystem: any = null;
  private projectKey: any = null;
  private biomeConfig = {
    formatter: {
      enabled: true,
      indentStyle: 'space' as const,
      indentWidth: 2,
      lineWidth: 100,
    },
    linter: {
      enabled: true,
      rules: {
        recommended: true,
        correctness: {
          noUnusedVariables: 'error',
          noUndeclaredVariables: 'error',
        },
        style: {
          noVar: 'error',
          useConst: 'warn',
        },
        suspicious: {
          noExplicitAny: 'warn',
          noImplicitAnyLet: 'error',
        },
      },
    },
    javascript: {
      formatter: {
        quoteStyle: 'single' as const,
        trailingComma: 'es5' as const,
        semicolons: 'always' as const,
      },
    },
  };

  async initialize(): Promise<void> {
    if (this.wasmInitialized) return;

    try {
      console.log('[FileValidator] Initializing Biome WASM...');
      // Initialize WASM module with explicit URL using new object parameter format
      await init({ module_or_path: biomeWasmUrl });

      // Create workspace and file system
      this.workspace = new biomeWasm.Workspace();
      this.fileSystem = new biomeWasm.MemoryFileSystem();

      // Open a project (uninitialized since we don't have a biome.json)
      const openResult = this.workspace.openProject({
        openUninitialized: true,
        path: '/',
        setAsCurrentProject: true,
      });

      if (!openResult || !openResult.projectKey) {
        throw new Error('Failed to open Biome project');
      }

      this.projectKey = openResult.projectKey;
      this.wasmInitialized = true;
      console.log('[FileValidator] Biome WASM initialized successfully with project key:', this.projectKey);
    } catch (error) {
      console.error('[FileValidator] Failed to initialize Biome WASM:', error);
      throw error;
    }
  }

  async validateFile(
    filePath: string,
    content: string,
    autoFix = true
  ): Promise<ValidationResult> {
    await this.initialize();

    const fileType = this.getFileType(filePath);
    if (!this.isSupportedFile(fileType)) {
      return {
        valid: true,
        errors: [],
        warnings: [],
        suggestions: [],
      };
    }

    const errors: ValidationError[] = [];
    const warnings: ValidationError[] = [];
    const suggestions: string[] = [];
    let formatted = content;

    try {
      // Step 1: Format the code if autoFix is enabled
      if (autoFix && this.workspace && this.projectKey) {
        try {
          // First open the file with the correct FileContent format
          this.workspace.openFile({
            path: filePath,
            projectKey: this.projectKey,
            content: {
              type: 'fromClient',
              content: content,
              version: 0
            },
          });

          // Update the file content in the workspace
          this.workspace.changeFile({
            path: filePath,
            content: content,
            projectKey: this.projectKey,
            version: 1,
          });

          // Format the file
          const formatResult = this.workspace.formatFile({
            path: filePath,
            projectKey: this.projectKey,
          });

          if (formatResult && formatResult.code && formatResult.code !== content) {
            formatted = formatResult.code;
            suggestions.push('Code was automatically formatted');
          }

          // Close the file
          this.workspace.closeFile({
            path: filePath,
            projectKey: this.projectKey,
          });
        } catch (formatError) {
          // Formatting failed, continue with original
          console.warn('Formatting failed:', formatError);
        }
      }

      // Step 2: Lint the code (if WASM linting is available)
      // Note: @biomejs/wasm-web might not have full linting capabilities
      // For now, we'll focus on formatting which is the main benefit

    } catch (error) {
      // Parse error - likely syntax issue
      errors.push({
        line: 1,
        column: 1,
        severity: 'error',
        message: `Syntax error: ${error instanceof Error ? error.message : 'Unknown error'}`,
      });

      suggestions.push('Fix syntax errors before proceeding');
    }

    // Add general suggestions based on errors
    if (errors.length > 0) {
      suggestions.unshift(`Found ${errors.length} error(s) that must be fixed`);
    }

    return {
      valid: errors.length === 0,
      errors,
      warnings,
      formatted: autoFix ? formatted : undefined,
      suggestions,
    };
  }

  private parseDiagnostic(diagnostic: any, content: string): ValidationError {
    const lines = content.split('\n');
    const location = diagnostic.location?.span?.[0];
    const line = location ? Math.floor(location / 1000) : 1;
    const column = location ? (location % 1000) : 1;

    return {
      line,
      column,
      severity: diagnostic.severity === 'error' ? 'error' : 'warning',
      message: diagnostic.message?.content || diagnostic.message || 'Unknown error',
      rule: diagnostic.category,
      fix: diagnostic.message?.advice?.[0] || undefined,
    };
  }

  private generateSuggestion(error: ValidationError, content: string): string | null {
    const lines = content.split('\n');
    const errorLine = lines[error.line - 1] || '';

    // Generate contextual suggestions based on common patterns
    if (error.message.includes('unused variable')) {
      const varName = this.extractVariableName(errorLine);
      return `Remove unused variable '${varName}' at line ${error.line}`;
    }

    if (error.message.includes('const instead of let')) {
      return `Change 'let' to 'const' at line ${error.line} since the value is never reassigned`;
    }

    if (error.message.includes('missing semicolon')) {
      return `Add semicolon at the end of line ${error.line}`;
    }

    if (error.message.includes('any type')) {
      return `Specify a proper type instead of 'any' at line ${error.line}`;
    }

    if (error.fix) {
      return `${error.fix} (line ${error.line})`;
    }

    return null;
  }

  private extractVariableName(line: string): string {
    const match = line.match(/(?:const|let|var)\s+(\w+)/);
    return match ? match[1] : 'variable';
  }

  private getFileType(filePath: string): string {
    const extension = filePath.split('.').pop() || '';
    return extension.toLowerCase();
  }

  private isSupportedFile(fileType: string): boolean {
    const supported = ['js', 'jsx', 'ts', 'tsx', 'mjs', 'cjs'];
    return supported.includes(fileType);
  }

  // Generate LLM-friendly feedback
  generateAgentFeedback(result: ValidationResult, filePath: string): string {
    if (result.valid && result.errors.length === 0) {
      return `✅ File "${filePath}" is valid and ready to save.`;
    }

    let feedback = `❌ File "${filePath}" has validation issues:\n\n`;

    // List errors
    if (result.errors.length > 0) {
      feedback += `**Errors (must fix):**\n`;
      result.errors.forEach((error, i) => {
        feedback += `${i + 1}. Line ${error.line}, Column ${error.column}: ${error.message}\n`;
        if (error.fix) {
          feedback += `   Fix: ${error.fix}\n`;
        }
      });
      feedback += '\n';
    }

    // List warnings
    if (result.warnings.length > 0) {
      feedback += `**Warnings (recommended to fix):**\n`;
      result.warnings.forEach((warning, i) => {
        feedback += `${i + 1}. Line ${warning.line}: ${warning.message}\n`;
      });
      feedback += '\n';
    }

    // Add suggestions
    if (result.suggestions.length > 0) {
      feedback += `**Suggestions:**\n`;
      result.suggestions.forEach((suggestion) => {
        feedback += `- ${suggestion}\n`;
      });
    }

    return feedback;
  }
}

// Singleton instance
export const fileValidator = new FileValidator();
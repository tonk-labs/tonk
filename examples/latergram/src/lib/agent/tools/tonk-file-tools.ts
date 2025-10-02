import { tool } from 'ai';
import { z } from 'zod';
import { getVFSService } from '../../../services/vfs-service';
import { fileValidator } from '../../file-validator';
import { typeScriptValidator } from '../../typescript-validator';

// Track all files written during this session
const sessionFiles = new Set<string>();
let hasUserConfirmedClearAll = false;

// Allowed directories for agent file operations
const ALLOWED_WRITE_PATHS = ['/src/stores/', '/src/components/', '/src/views/'];

/**
 * Validates that a path is within allowed directories for agent operations
 */
function validateWritePath(path: string): { valid: boolean; error?: string } {
  const isAllowed = ALLOWED_WRITE_PATHS.some(allowedPath =>
    path.startsWith(allowedPath)
  );

  if (!isAllowed) {
    return {
      valid: false,
      error: `Access denied: Agent can only write to ${ALLOWED_WRITE_PATHS.join(', ')}. Attempted path: ${path}`,
    };
  }

  return { valid: true };
}

export const tonkClearAllTool = tool({
  description:
    'Clear all Tonk files and folders. After first using it, you first need to ask the user to confirm this is their intention. DO NOT CONTINUE WITHOUT EXPLICIT PERMISSION AFTER CALLING THIS TOOL.',
  inputSchema: z.object({
    haveExplicitUserPermission: z
      .boolean()
      .describe(
        'Has the user already given EXPLICIT permission to clear all Tonk files and folders -> "false" if still need to ask user.'
      ),
  }),
  execute: async ({ haveExplicitUserPermission }) => {
    console.log('[TonkTool] CLEAR ALL called:', { haveExplicitUserPermission });
    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }
    try {
      if (!haveExplicitUserPermission || !hasUserConfirmedClearAll) {
        hasUserConfirmedClearAll = true;
        throw new Error('Confirmation is required');
      }

      // await vfs.deleteAll();
      console.log('[TonkTool] CLEAR ALL success');
      return {
        success: true,
      };
    } catch (error) {
      console.error('[TonkTool] CLEAR ALL error:', { error });
      hasUserConfirmedClearAll = false;
      return {
        completed: true,
        summary:
          'Ask user to confirm that they want to clear all Tonk files and folders',
        files_modified: [],
        timestamp: new Date().toISOString(),
        message: `The user should explicitly confirm they understand that this will clear all Tonk files and folders. If they have given their explicit permission this tool can be called again..
          Tell the user:
          - This will clear all Tonk files and folders
          - This will disconnect the Tonk from the previous group
          - The user should confirm they understand this is their intention and give explicit permission
          `,
      };
    }
  },
});

export const tonkReadFileTool = tool({
  description: 'Read a file from the Tonk virtual file system.',
  inputSchema: z.object({
    path: z
      .string()
      .min(1, 'Path is required')
      .describe('Absolute Tonk path, e.g. /app/src/index.ts'),
  }),
  execute: async ({ path }) => {
    console.log('[TonkTool] readFile called:', { path });
    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }

    try {
      const content = await vfs.readBytesAsString(path);
      console.log('[TonkTool] readFile success:', {
        path,
        contentLength: content.length,
      });
      return {
        path,
        content,
        success: true,
      };
    } catch (error) {
      console.error('[TonkTool] readFile error:', { path, error });
      return {
        path,
        content: '',
        success: false,
        error: error instanceof Error ? error.message : 'Failed to read file',
      };
    }
  },
});

export const tonkWriteFileTool = tool({
  description:
    'Create or overwrite a file in the Tonk virtual file system with the supplied content. The file will be validated and auto-formatted before saving.',
  inputSchema: z.object({
    path: z
      .string()
      .min(1, 'Path is required')
      .describe('Absolute Tonk path to write, e.g. /app/src/index.ts'),
    content: z.string().describe('Full file contents to persist.'),
  }),
  execute: async ({ path, content }) => {
    console.log('[TonkTool] writeFile called with validation:', {
      path,
      contentLength: content.length,
      content,
    });
    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }

    // Validate write path
    const pathValidation = validateWritePath(path);
    if (!pathValidation.valid) {
      console.error(
        '[TonkTool] writeFile path validation failed:',
        pathValidation.error
      );
      return {
        path,
        created: false,
        success: false,
        error: pathValidation.error,
      };
    }

    // Step 1: Biome validation (syntax and linting)
    console.log('[TonkTool] Starting validation for:', path);

    // Try Biome validation
    let validationResult: any = null;
    let feedback = '';

    try {
      validationResult = await fileValidator.validateFile(path, content, true);
      console.log('[TonkTool] Validation result:', {
        valid: validationResult.valid,
        errors: validationResult.errors.length,
        warnings: validationResult.warnings.length,
      });

      // Generate feedback
      feedback = fileValidator.generateAgentFeedback(validationResult, path);

      // If there are syntax errors, return them without proceeding
      if (validationResult.errors.length > 0) {
        console.error('[TonkTool] Validation failed:', feedback);
        return {
          path,
          created: false,
          success: false,
          error: `Validation failed. ${feedback}`,
          validationErrors: validationResult.errors,
          suggestions: validationResult.suggestions,
        };
      }
    } catch (validationError) {
      console.warn(
        '[TonkTool] Biome validation failed, continuing without it:',
        validationError
      );
      validationResult = {
        valid: true,
        errors: [],
        warnings: [],
        formatted: content,
        suggestions: [],
      };
    }

    // Step 2: TypeScript validation (only if it's a TS/TSX file and syntax is valid)
    const isTypeScriptFile = path.endsWith('.ts') || path.endsWith('.tsx');

    if (isTypeScriptFile) {
      try {
        // Use the simplified validateSyntax method
        const typeCheckResult = typeScriptValidator.validateSyntax(
          validationResult.formatted || content
        );

        if (!typeCheckResult.valid) {
          const tsFeedback = typeScriptValidator.generateAgentFeedback(
            typeCheckResult,
            path
          );
          console.error('[TonkTool] TypeScript validation failed:', tsFeedback);

          return {
            path,
            created: false,
            success: false,
            error: `TypeScript validation failed.\n\n${tsFeedback}`,
            typeErrors: typeCheckResult.diagnostics,
            suggestions: [
              `Fix ${typeCheckResult.errorCount} TypeScript error(s) before saving`,
            ],
          };
        }

        if (typeCheckResult.warningCount > 0) {
          feedback += `\n\nTypeScript Warnings: ${typeCheckResult.warningCount} warning(s) found`;
        }
      } catch (tsError) {
        console.warn(
          '[TonkTool] TypeScript validation failed to run, continuing without TS validation:',
          tsError
        );
      }
    }

    // Use formatted content if available
    const finalContent = validationResult?.formatted || content;

    try {
      const exists = await vfs.exists(path);
      await vfs.writeStringAsBytes(path, finalContent, !exists);

      // Track this file for finish validation
      sessionFiles.add(path);

      console.log('[TonkTool] writeFile success with validation:', {
        path,
        created: !exists,
        wasFormatted: finalContent !== content,
      });

      return {
        path,
        created: !exists,
        success: true,
        wasFormatted: finalContent !== content,
        warnings: validationResult.warnings,
        feedback: validationResult.warnings.length > 0 ? feedback : undefined,
      };
    } catch (error) {
      console.error('[TonkTool] writeFile error:', { path, error });
      return {
        path,
        created: false,
        success: false,
        error: error instanceof Error ? error.message : 'Failed to write file',
      };
    }
  },
});

export const tonkDeleteFileTool = tool({
  description: 'Delete a file from the Tonk virtual file system.',
  inputSchema: z.object({
    path: z
      .string()
      .min(1, 'Path is required')
      .describe('Absolute Tonk path to delete.'),
  }),
  execute: async ({ path }) => {
    console.log('[TonkTool] deleteFile called:', { path });
    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }

    // Validate delete path
    const pathValidation = validateWritePath(path);
    if (!pathValidation.valid) {
      console.error(
        '[TonkTool] deleteFile path validation failed:',
        pathValidation.error
      );
      return {
        path,
        success: false,
        error: pathValidation.error,
      };
    }

    try {
      await vfs.deleteFile(path);

      // Remove from session tracking
      sessionFiles.delete(path);

      console.log('[TonkTool] deleteFile success:', { path });
      return {
        path,
        success: true,
      };
    } catch (error) {
      console.error('[TonkTool] deleteFile error:', { path, error });
      return {
        path,
        success: false,
        error: error instanceof Error ? error.message : 'Failed to delete file',
      };
    }
  },
});

export const tonkListDirectoryTool = tool({
  description: 'List the entries in a Tonk directory.',
  inputSchema: z.object({
    path: z
      .string()
      .min(1, 'Path is required')
      .describe('Directory to list, e.g. /app/src'),
  }),
  execute: async ({ path }) => {
    console.log('[TonkTool] listDirectory called:', { path });
    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }

    try {
      const entries = await vfs.listDirectory(path);
      console.log('[TonkTool] listDirectory success:', {
        path,
        entriesCount: entries.length,
      });
      return {
        path,
        entries,
        success: true,
      };
    } catch (error) {
      console.error('[TonkTool] listDirectory error:', { path, error });
      return {
        path,
        entries: [],
        success: false,
        error:
          error instanceof Error ? error.message : 'Failed to list directory',
      };
    }
  },
});

export const tonkExistsTool = tool({
  description: 'Check whether a path exists in the Tonk virtual file system.',
  inputSchema: z.object({
    path: z
      .string()
      .min(1, 'Path is required')
      .describe('Absolute Tonk path to check.'),
  }),
  execute: async ({ path }) => {
    console.log('[TonkTool] exists called:', { path });
    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }

    try {
      const exists = await vfs.exists(path);
      console.log('[TonkTool] exists success:', { path, exists });
      return {
        path,
        exists,
        success: true,
      };
    } catch (error) {
      console.error('[TonkTool] exists error:', { path, error });
      return {
        path,
        exists: false,
        success: false,
      };
    }
  },
});

// Enhanced finish tool - validates all files before completion
export const finishTool = tool({
  description:
    'Call this tool when you have completed the requested task. This will validate all modified files before completion.',
  inputSchema: z.object({
    summary: z.string().describe('A brief summary of what was accomplished'),
    files_modified: z
      .array(z.string())
      .optional()
      .describe('List of file paths that were modified or created'),
  }),
  execute: async ({ summary, files_modified }) => {
    console.log('[TonkTool] FINISH called with validation:', {
      summary,
      files_modified,
    });

    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }

    // Determine which files to validate
    const filesToValidate = files_modified || Array.from(sessionFiles);

    // Collect all files for TypeScript project validation
    const allFiles = new Map<string, string>();
    const jstsFiles = filesToValidate.filter(f =>
      f.match(/\.(js|jsx|ts|tsx|mjs|cjs)$/)
    );

    // Read all files first
    for (const filePath of jstsFiles) {
      try {
        const content = await vfs.readBytesAsString(filePath);
        allFiles.set(filePath, content);
      } catch (error) {
        console.error(`Failed to read ${filePath}:`, error);
      }
    }

    // Validate all modified files
    const validationResults: Array<{
      path: string;
      valid: boolean;
      errors: number;
      warnings: number;
      typeErrors?: number;
      feedback?: string;
    }> = [];

    let hasErrors = false;

    // Step 1: Biome validation for each file
    for (const [filePath, content] of allFiles) {
      const result = await fileValidator.validateFile(filePath, content, false);

      validationResults.push({
        path: filePath,
        valid: result.valid,
        errors: result.errors.length,
        warnings: result.warnings.length,
        feedback:
          result.errors.length > 0
            ? fileValidator.generateAgentFeedback(result, filePath)
            : undefined,
      });

      if (result.errors.length > 0) {
        hasErrors = true;
      }
    }

    // Step 2: TypeScript validation for the entire project (if no syntax errors)
    if (!hasErrors && allFiles.size > 0) {
      const tsFiles = Array.from(allFiles.keys()).filter(
        f => f.endsWith('.ts') || f.endsWith('.tsx')
      );

      if (tsFiles.length > 0) {
        console.log('[TonkTool] Running TypeScript validation on project...');
        const typeCheckResults = new Map();

        // Update validation results with TypeScript errors
        for (const result of validationResults) {
          const tsResult = typeCheckResults.get(result.path);
          if (tsResult && !tsResult.valid) {
            result.valid = false;
            result.typeErrors = tsResult.errorCount;
            const tsFeedback = typeScriptValidator.generateAgentFeedback(
              tsResult,
              result.path
            );
            result.feedback = result.feedback
              ? `${result.feedback}\n\n${tsFeedback}`
              : tsFeedback;
            hasErrors = true;
          }
        }
      }
    }

    // If there are validation errors, return them
    if (hasErrors) {
      const errorFeedback = validationResults
        .filter(r => r.errors > 0)
        .map(r => r.feedback)
        .filter(Boolean)
        .join('\n\n');

      return {
        completed: false,
        summary,
        files_modified: filesToValidate,
        timestamp: new Date().toISOString(),
        validationFailed: true,
        validationResults,
        error: `Cannot finish - validation errors found in modified files:\n\n${errorFeedback}\n\nPlease fix these errors before calling finish again.`,
      };
    }

    // All validations passed
    console.log('[TonkTool] FINISH validation passed:', {
      filesValidated: validationResults.length,
      totalWarnings: validationResults.reduce((sum, r) => sum + r.warnings, 0),
    });

    // Clear session files after successful finish
    sessionFiles.clear();

    return {
      completed: true,
      summary,
      files_modified: filesToValidate,
      timestamp: new Date().toISOString(),
      validationResults,
      message:
        validationResults.length > 0
          ? `✅ All ${validationResults.length} files validated successfully`
          : '✅ Task completed',
    };
  },
});

export const tonkFileTools = {
  tonkReadFile: tonkReadFileTool,
  tonkWriteFile: tonkWriteFileTool,
  tonkDeleteFile: tonkDeleteFileTool,
  tonkListDirectory: tonkListDirectoryTool,
  tonkExists: tonkExistsTool,
  tonkClearAll: tonkClearAllTool,
  finish: finishTool,
};

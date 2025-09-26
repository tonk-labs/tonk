import { fileValidator, type ValidationResult } from './file-validator';

interface FileOperation {
  action: 'save' | 'finish';
  filePath: string;
  content: string;
}

interface FileOperationResult {
  success: boolean;
  filePath: string;
  validationResult?: ValidationResult;
  finalContent?: string;
  agentFeedback?: string;
}

export class AgentFileHandler {
  private fileStore = new Map<string, string>();

  async handleAgentFileOperation(
    operation: FileOperation
  ): Promise<FileOperationResult> {
    console.log(`Agent requesting to ${operation.action}: ${operation.filePath}`);

    // Step 1: Validate the file
    const validationResult = await fileValidator.validateFile(
      operation.filePath,
      operation.content,
      true // Enable auto-fix
    );

    // Step 2: Generate feedback for the agent
    const agentFeedback = fileValidator.generateAgentFeedback(
      validationResult,
      operation.filePath
    );

    // Step 3: Determine if we should proceed
    const shouldProceed = this.shouldProceedWithSave(validationResult, operation.action);

    if (!shouldProceed) {
      // Return failure with detailed feedback
      return {
        success: false,
        filePath: operation.filePath,
        validationResult,
        agentFeedback: this.createBlockingFeedback(validationResult, operation),
      };
    }

    // Step 4: Use formatted content if available
    const finalContent = validationResult.formatted || operation.content;

    // Step 5: Save the file
    if (operation.action === 'finish') {
      // Final save - stricter validation
      if (validationResult.errors.length > 0) {
        return {
          success: false,
          filePath: operation.filePath,
          validationResult,
          agentFeedback: `Cannot finish file with errors:\n${agentFeedback}`,
        };
      }
    }

    // Store the file (in real app, this would be actual file system)
    this.fileStore.set(operation.filePath, finalContent);

    return {
      success: true,
      filePath: operation.filePath,
      validationResult,
      finalContent,
      agentFeedback: validationResult.formatted !== operation.content
        ? `File saved with automatic formatting applied.\n${agentFeedback}`
        : agentFeedback,
    };
  }

  private shouldProceedWithSave(
    result: ValidationResult,
    action: 'save' | 'finish'
  ): boolean {
    // For 'finish' action, no errors allowed
    if (action === 'finish') {
      return result.errors.length === 0;
    }

    // For 'save' action, allow warnings but not errors
    return result.valid || result.errors.length === 0;
  }

  private createBlockingFeedback(
    result: ValidationResult,
    operation: FileOperation
  ): string {
    let feedback = `❌ Cannot ${operation.action} file "${operation.filePath}".\n\n`;

    feedback += `**Required fixes:**\n`;

    // Group errors by type for clearer instructions
    const errorGroups = this.groupErrorsByType(result.errors);

    for (const [errorType, errors] of Object.entries(errorGroups)) {
      feedback += `\n${errorType}:\n`;
      errors.forEach((error) => {
        feedback += `  - Line ${error.line}: ${error.message}\n`;
        if (error.fix) {
          feedback += `    → ${error.fix}\n`;
        }
      });
    }

    feedback += `\n**Instructions for agent:**\n`;
    feedback += `1. Fix all errors listed above\n`;
    feedback += `2. Pay special attention to:\n`;

    // Add specific instructions based on error types
    if (result.errors.some(e => e.message.includes('syntax'))) {
      feedback += `   - Check for missing brackets, semicolons, or quotes\n`;
    }
    if (result.errors.some(e => e.message.includes('type'))) {
      feedback += `   - Ensure all TypeScript types are properly defined\n`;
    }
    if (result.errors.some(e => e.message.includes('unused'))) {
      feedback += `   - Remove or use all declared variables\n`;
    }

    feedback += `3. Retry the ${operation.action} operation after fixing\n`;

    return feedback;
  }

  private groupErrorsByType(errors: ValidationResult['errors']) {
    const groups: Record<string, typeof errors> = {};

    for (const error of errors) {
      let category = 'Other Issues';

      if (error.message.includes('syntax')) {
        category = 'Syntax Errors';
      } else if (error.message.includes('type') || error.message.includes('Type')) {
        category = 'Type Errors';
      } else if (error.message.includes('unused') || error.message.includes('declared')) {
        category = 'Unused Code';
      } else if (error.message.includes('import') || error.message.includes('export')) {
        category = 'Module Issues';
      }

      if (!groups[category]) {
        groups[category] = [];
      }
      groups[category].push(error);
    }

    return groups;
  }

  // Get stored file content
  getFile(filePath: string): string | undefined {
    return this.fileStore.get(filePath);
  }

  // List all stored files
  listFiles(): string[] {
    return Array.from(this.fileStore.keys());
  }
}

// Export singleton
export const agentFileHandler = new AgentFileHandler();
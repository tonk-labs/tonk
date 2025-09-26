import { useState, useCallback } from 'react';
import { agentFileHandler } from '@/lib/agent-file-handler';

interface ValidationState {
  isValidating: boolean;
  lastResult: {
    success: boolean;
    feedback: string;
    filePath: string;
  } | null;
  error: Error | null;
}

export function useAgentFileValidation() {
  const [state, setState] = useState<ValidationState>({
    isValidating: false,
    lastResult: null,
    error: null,
  });

  const validateAndSave = useCallback(
    async (filePath: string, content: string, action: 'save' | 'finish' = 'save') => {
      setState((prev) => ({ ...prev, isValidating: true, error: null }));

      try {
        const result = await agentFileHandler.handleAgentFileOperation({
          action,
          filePath,
          content,
        });

        setState({
          isValidating: false,
          lastResult: {
            success: result.success,
            feedback: result.agentFeedback || '',
            filePath: result.filePath,
          },
          error: null,
        });

        // Return result for agent to process
        return {
          success: result.success,
          content: result.finalContent || content,
          feedback: result.agentFeedback,
          errors: result.validationResult?.errors || [],
          warnings: result.validationResult?.warnings || [],
        };
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error';

        setState({
          isValidating: false,
          lastResult: {
            success: false,
            feedback: `Validation failed: ${errorMessage}`,
            filePath,
          },
          error: error instanceof Error ? error : new Error(errorMessage),
        });

        return {
          success: false,
          content,
          feedback: `Validation system error: ${errorMessage}. Please check the file manually.`,
          errors: [],
          warnings: [],
        };
      }
    },
    []
  );

  const getStoredFile = useCallback((filePath: string) => {
    return agentFileHandler.getFile(filePath);
  }, []);

  const listStoredFiles = useCallback(() => {
    return agentFileHandler.listFiles();
  }, []);

  return {
    validateAndSave,
    getStoredFile,
    listStoredFiles,
    isValidating: state.isValidating,
    lastResult: state.lastResult,
    error: state.error,
  };
}
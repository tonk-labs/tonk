import React from 'react';
import { useAgentFileValidation } from '@/hooks/useAgentFileValidation';

// Example component showing how to integrate with your LLM agent
export function AgentFileValidationExample() {
  const { validateAndSave, lastResult, isValidating } = useAgentFileValidation();

  // Example: Agent attempts to save a file with issues
  const handleAgentSaveAttempt = async () => {
    // Simulating agent trying to save problematic code
    const problematicCode = `
      // This code has several issues
      let unusedVariable = 42;

      function processData(data) {  // Missing type annotation
        console.log("Processing data")  // Missing semicolon
        let result = data.map(item => {
          return item * 2
        })
        return result;  // Should use const instead of let
      }

      // Missing export
      processData([1, 2, 3])
    `;

    const result = await validateAndSave(
      '/src/utils/data-processor.ts',
      problematicCode,
      'save'
    );

    // This is what the agent would receive back
    console.log('Agent Save Result:', {
      success: result.success,
      feedback: result.feedback,
      shouldRetry: !result.success && result.errors.length > 0,
    });

    // Agent could parse the feedback and attempt to fix
    if (!result.success && result.errors.length > 0) {
      console.log('Agent should fix these issues:', result.errors);
      // In real implementation, agent would modify code and retry
    }
  };

  // Example: Agent attempts to finish/finalize a file
  const handleAgentFinishAttempt = async () => {
    const cleanCode = `
export function processData(data: number[]): number[] {
  console.log('Processing data');
  const result = data.map(item => {
    return item * 2;
  });
  return result;
}

// Usage
const processed = processData([1, 2, 3]);
console.log(processed);
`;

    const result = await validateAndSave(
      '/src/utils/data-processor.ts',
      cleanCode,
      'finish'
    );

    console.log('Agent Finish Result:', {
      success: result.success,
      finalContent: result.content,
      feedback: result.feedback,
    });
  };

  return (
    <div className="p-4 space-y-4">
      <h2 className="text-xl font-bold">Agent File Validation Demo</h2>

      <div className="space-x-4">
        <button
          onClick={handleAgentSaveAttempt}
          disabled={isValidating}
          className="px-4 py-2 bg-blue-500 text-white rounded disabled:opacity-50"
        >
          Simulate Agent Save (with errors)
        </button>

        <button
          onClick={handleAgentFinishAttempt}
          disabled={isValidating}
          className="px-4 py-2 bg-green-500 text-white rounded disabled:opacity-50"
        >
          Simulate Agent Finish (clean code)
        </button>
      </div>

      {isValidating && (
        <div className="text-blue-600">Validating file...</div>
      )}

      {lastResult && (
        <div className={`p-4 rounded ${lastResult.success ? 'bg-green-50' : 'bg-red-50'}`}>
          <h3 className="font-semibold mb-2">
            {lastResult.success ? '✅ Success' : '❌ Validation Failed'}
          </h3>
          <pre className="whitespace-pre-wrap text-sm">
            {lastResult.feedback}
          </pre>
        </div>
      )}
    </div>
  );
}
import { createOpenRouter } from '@openrouter/ai-sdk-provider';
import { tonkFileTools } from './tools/tonk-file-tools';
import { tonkManualTool } from './tools/tonk-manpage';

// Get OpenRouter API key from environment
const getOpenRouterApiKey = (): string => {
  const apiKey = import.meta.env.VITE_OPENROUTER_API_KEY;

  if (!apiKey) {
    throw new Error(
      'VITE_OPENROUTER_API_KEY is required. Please set VITE_OPENROUTER_API_KEY in your .env file.'
    );
  }

  return apiKey;
};

// Create OpenRouter provider
export const createOpenRouterProvider = () => {
  return createOpenRouter({
    apiKey: getOpenRouterApiKey(),
  });
};

// Model configuration
export const MODEL = 'anthropic/claude-sonnet-4';

// Agent system prompt
export const AGENT_SYSTEM_PROMPT = `You are a helpful coding agent working on the Latergram project.

You have direct access to the Tonk virtual file system via your tools. Use them to:
- inspect directories before making changes
- read files fully before modifying them
- write the entire updated file content when saving changes (no diffs)
- create new files when asked by providing the complete contents
- confirm whether a path exists if you are uncertain

CRITICAL: ALWAYS START BY READING THE MANUAL
Before implementing ANY code (components, pages, or stores), you MUST call the tonkManualTool to read the development guidelines. This contains all the rules, patterns, and error solutions you need.

To complete your task, you MUST call the 'finish' tool when you have completed the requested task. The finish tool should include:
- A summary of what you accomplished
- A list of files you modified or created

Remember: You MUST call the 'finish' tool when done with the task. Do not leave tasks incomplete.
`;

const tonkTools = {
  ...tonkFileTools,
  tonkManualTool: tonkManualTool,
};

// Export tools for use in agent service
export { tonkTools };

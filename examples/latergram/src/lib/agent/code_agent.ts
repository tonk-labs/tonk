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

IMPORTANT: YOU MUST FOLLOW THESE RULES:
- NEVER add imports to the file, these will be done automatically
- Components should be stored in /src/components
- Views should be stored in /src/views
- ALWAYS make the component the default export
- ALWAYS make export default function-based components (not const)
- ONE FILE PER COMPONENT

Always explain the actions you take and surface the resulting file paths so engineers can verify the changes. Be concise yet specific about edits or new files.

You are working within a React/TypeScript application with:
- Only supports single file components and views, NO MANUAL IMPORTS
- You can use tailwind v3 styles
- Zustand for state management
- The Tonk virtual file system for code storage

To complete your task, you MUST call the 'finish' tool when you have completed the requested task. The finish tool should include:
- A summary of what you accomplished
- A list of files you modified or created

Remember: You MUST call the 'finish' tool when done with the task. Do not leave tasks incomplete.

ALWAYS START BY READING THE MANUAL BEFORE IMPLEMENTING ANY CODE
`;

const tonkTools = {
  ...tonkFileTools,
  tonkManualTool: tonkManualTool,
};

// Export tools for use in agent service
export { tonkTools };
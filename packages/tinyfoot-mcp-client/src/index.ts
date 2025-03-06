#!/usr/bin/env node

import {Server} from '@modelcontextprotocol/sdk/server/index.js';
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
  ToolSchema,
} from '@modelcontextprotocol/sdk/types.js';
import {z} from 'zod';
import {zodToJsonSchema} from 'zod-to-json-schema';
import {StdioServerTransport} from '@modelcontextprotocol/sdk/server/stdio.js';
import {ProjectInstructions} from './projectSchema.js';

// Store allowed directories in normalized form
// Schema definition for createTask
const TinyfootTaskSchema = z.object({
  description: z
    .string()
    .describe(
      'A detailed description of the feature or functionality you want to implement. This will be used to find relevant modules and provide implementation guidance.',
    ),
  context: z
    .string()
    .optional()
    .describe(
      'Additional context about the feature request, such as specific requirements, constraints, or related features.',
    ),
});

const ToolInputSchema = ToolSchema.shape.inputSchema;
type ToolInput = z.infer<typeof ToolInputSchema>;

// Server setup
const server = new Server(
  {
    name: 'tinyfoot-mcp',
    version: '0.1.1',
  },
  {
    capabilities: {
      tools: {},
    },
  },
);

// Tool handlers
server.setRequestHandler(ListToolsRequestSchema, async () => {
  return {
    tools: [
      {
        name: 'newTask',
        description:
          'Analyzes a user request and provides implementation guidance by finding relevant modules and providing project structure information. Use this whenever a user wants to create a new tinyfoot task.',
        inputSchema: zodToJsonSchema(TinyfootTaskSchema) as ToolInput,
      },
    ],
  };
});

server.setRequestHandler(CallToolRequestSchema, async request => {
  try {
    const {name} = request.params;
    const args = request.params.arguments;

    if (!args || typeof args !== 'object') {
      throw new Error('Arguments must be an object');
    }

    const description = args.description;
    if (!description || typeof description !== 'string') {
      throw new Error('Description is required and must be a string');
    }

    switch (name) {
      case 'newTask': {
        try {
          // Find similar modules by querying the server
          const response = await fetch('http://localhost:4321/find-similar', {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json',
            },
            body: JSON.stringify({
              query: description,
              threshold: 0.3,
            }),
          });

          if (!response.ok) {
            throw new Error(`Server request failed: ${response.statusText}`);
          }

          const {results: similarModules} = await response.json();

          // Get project schema
          const projectSchema = new ProjectInstructions();

          return {
            content: [
              {
                type: 'text',
                text: `
STEP 1: PROJECT STRUCTURE
   - Components: Components should handle most render logic. ${
     projectSchema.components
   }
   - Modules: Modules should be anything not covered by components, stores or views. ${
     projectSchema.modules
   }
   - Stores: Stores can be either for local state (e.g. UI) or for sync-engine like multiplayer state (e.g. keepsync). ${
     projectSchema.stores
   }
   - Views: Views are like pages, it's where everything comes together. Components are laid out with modules and stores. ${
     projectSchema.views
   }

STEP 2: REQUIRED ACTIONS
  1. Summarize the feature request
  2. Ask user for next steps
  3. Wait for user confirmation before writing out the task 

   DO NOT PROCEED WITH IMPLEMENTATION UNTIL STEPS 1-2 ARE COMPLETED

  CHECKPOINT 1: ✋ Stop and summarize feature
  CHECKPOINT 2: ✋ Stop and get user input
  CHECKPOINT 3: ✋ Stop and confirm approach

STEP 3: Output the task as a json file with all the necessary technical information for an LLM to implement the task correctly.

  Context:

  Similar modules ${similarModules.map(
    (m: {name: string; description: string}) => m.name + ': ' + m.description,
  )}
            `,
              },
            ],
          };
        } catch (error) {
          const errorMessage =
            error instanceof Error ? error.message : String(error);
          return {
            content: [
              {
                type: 'text',
                text: `Error processing task: ${errorMessage}`,
              },
            ],
            isError: true,
          };
        }
      }

      default:
        throw new Error(`Unknown tool: ${name}`);
    }
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error);
    return {
      content: [{type: 'text', text: `Error: ${errorMessage}`}],
      isError: true,
    };
  }
});

// Connect using STDIO transport
const transport = new StdioServerTransport();

async function main() {
  try {
    await server.connect(transport);
  } catch (error) {
    console.error('Failed to connect server:', error);
    process.exit(1);
  }
}

void main();

// Cleanup on exit
process.on('SIGINT', () => {
  process.exit();
});

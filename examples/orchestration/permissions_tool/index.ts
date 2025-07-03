#!/usr/bin/env node

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
  CallToolRequest,
  ListToolsRequest,
} from "@modelcontextprotocol/sdk/types.js";
import * as readline from "readline";

const server = new Server(
  {
    name: "permissions-server",
    version: "1.0.0",
  },
  {
    capabilities: {
      tools: {},
    },
  }
);

// Function to get user input from command line
async function getUserPermission(prompt: string): Promise<boolean> {
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  });

  return new Promise((resolve) => {
    rl.question(`${prompt} (y/n): `, (answer) => {
      rl.close();
      const response = answer.toLowerCase().trim();
      resolve(response === "y" || response === "yes");
    });
  });
}

// List available tools
server.setRequestHandler(ListToolsRequestSchema, async () => {
  return {
    tools: [
      {
        name: "approval_prompt",
        description:
          "Prompts the user for permission approval via command line",
        inputSchema: {
          type: "object",
          properties: {
            tool_name: {
              type: "string",
              description: "The tool requesting permission",
            },
            input: {
              type: "object",
              description: "The input for the tool",
              additionalProperties: true,
            },
          },
          required: ["tool_name", "input"],
        },
      },
    ],
  };
});

// Handle tool calls
server.setRequestHandler(CallToolRequestSchema, async (request) => {
  if (request.params.name === "approval_prompt") {
    const { tool_name, input } = request.params.arguments as {
      tool_name: string;
      input: Record<string, any>;
    };

    // Format the prompt message
    const promptText = `Allow ${tool_name} with input: ${JSON.stringify(input)}?`;

    try {
      // Get user permission
      const approved = await getUserPermission(promptText);

      return {
        content: [
          {
            type: "text",
            text: JSON.stringify(
              approved
                ? {
                    behavior: "allow",
                    updatedInput: input,
                  }
                : {
                    behavior: "deny",
                    message: "Permission denied by user",
                  }
            ),
          },
        ],
      };
    } catch (error) {
      return {
        content: [
          {
            type: "text",
            text: JSON.stringify({
              behavior: "deny",
              message: `Error getting user permission: ${error instanceof Error ? error.message : String(error)}`,
            }),
          },
        ],
        isError: true,
      };
    }
  } else {
    throw new Error(`Unknown tool: ${request.params.name}`);
  }
});

// Start the server
async function main(): Promise<void> {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  console.error("Permissions MCP server running on stdio");
}

main().catch((error) => {
  console.error("Server error:", error);
  process.exit(1);
});

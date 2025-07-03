import { spawn, ChildProcess } from "child_process";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import type { Tool } from "@modelcontextprotocol/sdk/types.js";
import type Anthropic from "@anthropic-ai/sdk";
import { MCPServerConfig } from "./claudeApiProvider";

/**
 * MCP Server instance
 */
interface MCPServer {
  name: string;
  process: ChildProcess | null;
  client: Client;
  transport: StdioClientTransport;
  tools: Tool[];
}

/**
 * MCP Client for managing MCP servers and tools
 */
export class McpClient {
  private servers: Map<string, MCPServer> = new Map();
  private isInitialized = false;

  constructor(private config: MCPServerConfig) {}

  /**
   * Initialize MCP servers based on configuration
   */
  async initializeServers(config: MCPServerConfig): Promise<void> {
    if (this.isInitialized) {
      return;
    }

    // Normalize configuration - handle both flat and nested formats
    let servers: Record<string, { command: string; args: string[]; env?: Record<string, string>; cwd?: string }> = {};
    
    if (config.mcpServers) {
      // Nested format: { mcpServers: { serverName: { ... } } }
      servers = config.mcpServers;
    } else {
      // Flat format: { serverName: { ... } }
      servers = Object.fromEntries(
        Object.entries(config).filter(([_, value]) => 
          value && typeof value === 'object' && value.command && value.args
        )
      );
    }

    // Only start servers if there are valid configurations
    const serverEntries = Object.entries(servers).filter(([_, serverConfig]) => 
      serverConfig && serverConfig.command && serverConfig.args
    );

    if (serverEntries.length === 0) {
      console.log('📝 No MCP servers configured');
      this.isInitialized = true;
      return;
    }

    for (const [serverName, serverConfig] of serverEntries) {
      try {
        await this.startServer(serverName, serverConfig);
      } catch (error) {
        console.error(`❌ Failed to start MCP server ${serverName}:`, error);
      }
    }

    this.isInitialized = true;
  }

  /**
   * Start a single MCP server
   */
  private async startServer(
    name: string,
    config: { command: string; args: string[]; env?: Record<string, string>; cwd?: string },
  ): Promise<void> {
    console.log(`🚀 Starting MCP server: ${name}`);

    // Create transport and client
    const transportParams: any = {
      command: config.command,
      args: config.args,
    };

    if (config.env) {
      transportParams.env = config.env;
    }

    if (config.cwd) {
      transportParams.cwd = config.cwd;
    }

    const transport = new StdioClientTransport(transportParams);

    const client = new Client(
      {
        name: `ai-worker-${name}`,
        version: "1.0.0",
      },
      {
        capabilities: {
          tools: {},
        },
      },
    );

    try {
      // Connect to the server
      await client.connect(transport);
      console.log(`✅ Connected to MCP server: ${name}`);

      // List available tools
      const toolsResponse = await client.listTools();
      const tools = toolsResponse.tools || [];

      console.log(
        `🔧 MCP server ${name} tools:`,
        tools.map((t) => t.name),
      );

      // Store server information
      this.servers.set(name, {
        name,
        process: null, // Transport handles the process
        client,
        transport,
        tools,
      });
    } catch (error) {
      console.error(`❌ Failed to connect to MCP server ${name}:`, error);
      await transport.close();
      throw error;
    }
  }

  /**
   * Get all available tools from all servers
   */
  async getAvailableTools(): Promise<Anthropic.Tool[]> {
    const allTools: Anthropic.Tool[] = [];

    for (const server of this.servers.values()) {
      for (const tool of server.tools) {
        // Convert MCP tool to Anthropic tool format
        const anthropicTool: Anthropic.Tool = {
          name: `${server.name}_${tool.name}`,
          description:
            tool.description || `Tool ${tool.name} from ${server.name}`,
          input_schema:
            tool.inputSchema &&
            typeof tool.inputSchema === "object" &&
            (tool.inputSchema as any).type
              ? (tool.inputSchema as Anthropic.Tool.InputSchema)
              : {
                  type: "object" as const,
                  properties: {},
                },
        };

        allTools.push(anthropicTool);
      }
    }

    return allTools;
  }

  /**
   * Call a tool on the appropriate MCP server
   */
  async callTool(toolName: string, input: any): Promise<string> {
    // Parse server name and tool name
    const parts = toolName.split("_");
    if (parts.length < 2) {
      throw new Error(
        `Invalid tool name format: ${toolName}. Expected format: serverName_toolName`,
      );
    }

    const serverName = parts[0] || '';
    const actualToolName = parts.slice(1).join("_");

    const server = this.servers.get(serverName);
    if (!server) {
      throw new Error(`MCP server ${serverName} not found`);
    }

    try {
      console.log(
        `🔧 Calling tool ${actualToolName} on server ${serverName} with input:`,
        input,
      );

      const response = await server.client.callTool({
        name: actualToolName,
        arguments: input,
      });

      console.log(`🔍 Tool response:`, JSON.stringify(response, null, 2));

      if (response.isError) {
        // Extract error message from response content if available
        let errorMessage = response.error;
        if (!errorMessage && response.content && Array.isArray(response.content)) {
          const errorContent = response.content.find((c: any) => c.type === 'text');
          if (errorContent) {
            errorMessage = errorContent.text;
          }
        }
        if (!errorMessage) {
          errorMessage = JSON.stringify(response);
        }
        
        console.warn(`⚠️ MCP tool ${actualToolName} returned error:`, errorMessage);
        throw new Error(String(errorMessage));
      }

      // Format the result
      let result = "";
      if (response.content && Array.isArray(response.content)) {
        for (const content of response.content) {
          if (content.type === "text") {
            result += content.text;
          } else if (content.type === "image") {
            result += `[Image: ${content.mimeType || "unknown"}]`;
          } else if (content.type === "resource") {
            result += `[Resource: ${content.resource?.uri || "unknown"}]`;
          }
        }
      }

      console.log(`✅ Tool ${actualToolName} result:`, result);
      return result;
    } catch (error) {
      console.error(`❌ Tool call failed for ${toolName}:`, error);
      throw error;
    }
  }

  /**
   * Check if a specific server is running
   */
  isServerRunning(serverName: string): boolean {
    return this.servers.has(serverName);
  }

  /**
   * Get list of running servers
   */
  getRunningServers(): string[] {
    return Array.from(this.servers.keys());
  }

  /**
   * Stop a specific server
   */
  async stopServer(serverName: string): Promise<void> {
    const server = this.servers.get(serverName);
    if (!server) {
      return;
    }

    try {
      await server.client.close();
      await server.transport.close();
      if (server.process) {
        server.process.kill();
      }
      this.servers.delete(serverName);
      console.log(`🛑 Stopped MCP server: ${serverName}`);
    } catch (error) {
      console.error(`❌ Error stopping MCP server ${serverName}:`, error);
    }
  }

  /**
   * Cleanup all servers
   */
  async cleanup(): Promise<void> {
    console.log("🧹 Cleaning up MCP servers...");

    const cleanupPromises = Array.from(this.servers.keys()).map((serverName) =>
      this.stopServer(serverName),
    );

    await Promise.all(cleanupPromises);
    this.servers.clear();
    this.isInitialized = false;

    console.log("✅ MCP servers cleanup completed");
  }
}


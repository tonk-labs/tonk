import Anthropic from '@anthropic-ai/sdk';
import { McpClient } from './mcpClient';

/**
 * Claude API native request interface
 */
export interface LLMRequest {
  prompt: string;
  systemPrompt?: string;
  maxTurns?: number;
  allowedTools?: string[];
  disallowedTools?: string[];
  mcpConfig?: MCPServerConfig;
  permissionMode?: "default" | "acceptEdits" | "bypassPermissions" | "plan";
  workingDirectory?: string;
  verbose?: boolean;
  outputFormat?: "text" | "json" | "stream-json";
  abortController?: AbortController;
}

/**
 * Claude API native response interface
 */
export interface LLMResponse {
  content: string;
  totalCostUsd?: number;
  sessionId?: string;
  provider: string;
  success: boolean;
  error?: string;
}

/**
 * MCP Server configuration - can be either flat or nested under mcpServers
 */
export interface MCPServerConfig {
  mcpServers?: {
    [serverName: string]: {
      command: string;
      args: string[];
      env?: Record<string, string>;
      cwd?: string;
    };
  };
  // Also support flat structure for backwards compatibility
  [serverName: string]: any;
}

/**
 * Claude API configuration options
 */
export interface ClaudeApiOptions {
  apiKey?: string;
  systemPrompt?: string;
  maxTurns?: number;
  allowedTools?: string[];
  disallowedTools?: string[];
  mcpConfig?: MCPServerConfig;
  permissionMode?: "default" | "acceptEdits" | "bypassPermissions" | "plan";
  workingDirectory?: string;
  verbose?: boolean;
  outputFormat?: "text" | "json" | "stream-json";
  model?: string;
  maxTokens?: number;
}

/**
 * Claude API provider implementation
 */
export class ClaudeApiProvider {
  name = "claude-api";
  private client: Anthropic;
  private mcpClient: McpClient;

  constructor(private options: ClaudeApiOptions = {}) {
    this.client = new Anthropic({
      apiKey: options.apiKey || process.env.ANTHROPIC_API_KEY,
    });
    // Only initialize McpClient with explicit config, not default empty config
    this.mcpClient = new McpClient({});
  }

  /**
   * Check if the provider is configured and ready to use
   */
  isConfigured(): boolean {
    return !!(this.options.apiKey || process.env.ANTHROPIC_API_KEY);
  }

  async complete(request: LLMRequest): Promise<LLMResponse> {
    if (!this.isConfigured()) {
      return {
        content: "",
        provider: this.name,
        success: false,
        error: "Anthropic API key is not configured. Please set ANTHROPIC_API_KEY environment variable.",
      };
    }

    try {
      // Initialize MCP servers if needed
      await this.mcpClient.initializeServers(request.mcpConfig || this.options.mcpConfig || {});

      // Get available tools
      const availableTools = await this.mcpClient.getAvailableTools();
      const allowedTools = this.filterTools(availableTools, request.allowedTools, request.disallowedTools);

      // Build messages
      const messages = this.buildMessages(request);
      
      // Make the API call
      const response = await this.client.messages.create({
        model: this.options.model || "claude-3-5-sonnet-20241022",
        max_tokens: this.options.maxTokens || 4096,
        messages,
        system: request.systemPrompt || this.options.systemPrompt,
        tools: allowedTools.length > 0 ? allowedTools : undefined,
      });

      // Handle tool use if present
      let finalContent = "";
      let totalCost = 0;

      if (response.content) {
        for (const content of response.content) {
          if (content.type === "text") {
            finalContent += content.text;
          } else if (content.type === "tool_use") {
            const toolResult = await this.mcpClient.callTool(content.name, content.input);
            finalContent += `\n[Tool: ${content.name}]\n${toolResult}\n`;
          }
        }
      }

      // Calculate cost (approximate)
      if (response.usage) {
        totalCost = this.calculateCost(response.usage);
      }

      return {
        content: finalContent,
        totalCostUsd: totalCost,
        sessionId: response.id,
        provider: this.name,
        success: true,
      };
    } catch (error) {
      console.error("❌ Claude API error:", error);
      return {
        content: "",
        provider: this.name,
        success: false,
        error: `Claude API call failed: ${error instanceof Error ? error.message : String(error)}`,
      };
    }
  }

  async *stream(request: LLMRequest): AsyncIterable<string> {
    if (!this.isConfigured()) {
      throw new Error("Anthropic API key is not configured. Please set ANTHROPIC_API_KEY environment variable.");
    }

    try {
      // Initialize MCP servers if needed
      await this.mcpClient.initializeServers(request.mcpConfig || this.options.mcpConfig || {});

      // Get available tools
      const availableTools = await this.mcpClient.getAvailableTools();
      const allowedTools = this.filterTools(availableTools, request.allowedTools, request.disallowedTools);

      // Build messages
      const messages = this.buildMessages(request);
      
      // Make the streaming API call
      const stream = await this.client.messages.stream({
        model: this.options.model || "claude-3-5-sonnet-20241022",
        max_tokens: this.options.maxTokens || 4096,
        messages,
        system: request.systemPrompt || this.options.systemPrompt,
        tools: allowedTools.length > 0 ? allowedTools : undefined,
      });

      for await (const chunk of stream) {
        if (chunk.type === "content_block_delta" && chunk.delta.type === "text_delta") {
          yield chunk.delta.text;
        } else if (chunk.type === "content_block_start" && chunk.content_block.type === "tool_use") {
          // Handle tool use in streaming
          const toolResult = await this.mcpClient.callTool(
            chunk.content_block.name,
            chunk.content_block.input
          );
          yield `\n[Tool: ${chunk.content_block.name}]\n${toolResult}\n`;
        }
      }
    } catch (error) {
      throw new Error(
        `Claude API stream failed: ${error instanceof Error ? error.message : String(error)}`
      );
    }
  }

  /**
   * Build Claude API messages from request
   */
  private buildMessages(request: LLMRequest): Anthropic.Messages.MessageParam[] {
    return [
      {
        role: "user",
        content: request.prompt,
      },
    ];
  }

  /**
   * Filter tools based on allowed/disallowed lists
   */
  private filterTools(
    availableTools: Anthropic.Tool[],
    allowedTools?: string[],
    disallowedTools?: string[]
  ): Anthropic.Tool[] {
    let filteredTools = availableTools;

    if (allowedTools && allowedTools.length > 0) {
      filteredTools = filteredTools.filter(tool => 
        allowedTools.includes(tool.name) || 
        (tool.name.includes('_') && allowedTools.includes(tool.name.split('_')[0]))
      );
    }

    if (disallowedTools && disallowedTools.length > 0) {
      filteredTools = filteredTools.filter(tool => 
        !disallowedTools.includes(tool.name) && 
        !(tool.name.includes('_') && disallowedTools.includes(tool.name.split('_')[0]))
      );
    }

    return filteredTools;
  }

  /**
   * Calculate approximate cost based on usage
   */
  private calculateCost(usage: Anthropic.Usage): number {
    // Claude 3.5 Sonnet pricing (approximate)
    const inputCostPer1K = 0.003; // $0.003 per 1K input tokens
    const outputCostPer1K = 0.015; // $0.015 per 1K output tokens
    
    const inputCost = (usage.input_tokens / 1000) * inputCostPer1K;
    const outputCost = (usage.output_tokens / 1000) * outputCostPer1K;
    
    return inputCost + outputCost;
  }

  /**
   * Cleanup resources
   */
  async cleanup(): Promise<void> {
    await this.mcpClient.cleanup();
  }
}
import Anthropic from "@anthropic-ai/sdk";
import { McpClient } from "./mcpClient";

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
  conversationHistory?: Array<{ role: "user" | "assistant"; content: string }>;
  userInput?: string; // For responses to user prompts
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
  isFinished?: boolean;
  needsUserInput?: boolean;
  userPrompt?: string;
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
        error:
          "Anthropic API key is not configured. Please set ANTHROPIC_API_KEY environment variable.",
      };
    }

    try {
      // Initialize MCP servers if needed
      await this.mcpClient.initializeServers(
        request.mcpConfig || this.options.mcpConfig || {},
      );

      // Get available tools
      const availableTools = await this.mcpClient.getAvailableTools();
      const allowedTools = this.filterTools(
        availableTools,
        request.allowedTools,
        request.disallowedTools,
      );

      // Build messages - this includes conversation history
      const messages = this.buildMessages(request);

      let finalContent = "";
      let totalCost = 0;
      let lastResponse: any = null;

      // Handle multi-turn conversation with tool use
      let currentMessages = [...messages];
      let toolUseIteration = 0;
      const maxToolIterations = 5;

      while (toolUseIteration < maxToolIterations) {
        // Make the API call
        const createParams: any = {
          model: this.options.model || "claude-3-5-sonnet-20241022",
          max_tokens: this.options.maxTokens || 4096,
          messages: currentMessages,
        };

        if (request.systemPrompt || this.options.systemPrompt) {
          createParams.system = request.systemPrompt || this.options.systemPrompt;
        }

        if (allowedTools.length > 0) {
          createParams.tools = allowedTools;
        }

        const response = await this.client.messages.create(createParams);
        lastResponse = response;

        // Calculate cost (approximate)
        if (response.usage) {
          totalCost += this.calculateCost(response.usage);
        }

        let hasToolUse = false;
        let textContent = "";
        const toolResults: Array<{ tool_use_id: string; content: string }> = [];

        if (response.content) {
          for (const content of response.content) {
            if (content.type === "text") {
              textContent += content.text;
            } else if (content.type === "tool_use") {
              hasToolUse = true;
              try {
                const toolResult = await this.mcpClient.callTool(
                  content.name,
                  content.input,
                );
                toolResults.push({
                  tool_use_id: content.id,
                  content: toolResult,
                });
              } catch (error) {
                // Log the error but don't fail the entire request
                console.warn(`⚠️ Tool call failed for ${content.name}:`, error);
                toolResults.push({
                  tool_use_id: content.id,
                  content: `Error: ${error instanceof Error ? error.message : String(error)}`,
                });
              }
            }
          }
        }

        // Add assistant response to conversation (only if it has content)
        if (response.content && response.content.length > 0) {
          currentMessages.push({
            role: "assistant",
            content: response.content,
          });
        }

        // Add text content to final content
        finalContent += textContent;
        
        // Include tool results in final content for conversation history
        if (toolResults.length > 0) {
          for (const toolResult of toolResults) {
            finalContent += `\n[Tool Result: ${toolResult.content}]\n`;
          }
        }

        // If no tool use, we're done
        if (!hasToolUse) {
          break;
        }

        // Add tool results as user messages
        if (toolResults.length > 0) {
          currentMessages.push({
            role: "user",
            content: toolResults.map(result => ({
              type: "tool_result" as const,
              tool_use_id: result.tool_use_id,
              content: result.content,
            })),
          });
        }

        toolUseIteration++;
      }

      // Parse special control signals from AI response
      const controlSignals = this.parseControlSignals(finalContent);

      return {
        content: finalContent,
        totalCostUsd: totalCost,
        sessionId: lastResponse?.id,
        provider: this.name,
        success: true,
        // Add control signals
        ...(controlSignals.isFinished !== undefined && {
          isFinished: controlSignals.isFinished,
        }),
        ...(controlSignals.needsUserInput !== undefined && {
          needsUserInput: controlSignals.needsUserInput,
        }),
        ...(controlSignals.userPrompt !== undefined && {
          userPrompt: controlSignals.userPrompt,
        }),
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
      throw new Error(
        "Anthropic API key is not configured. Please set ANTHROPIC_API_KEY environment variable.",
      );
    }

    try {
      // Initialize MCP servers if needed
      await this.mcpClient.initializeServers(
        request.mcpConfig || this.options.mcpConfig || {},
      );

      // Get available tools
      const availableTools = await this.mcpClient.getAvailableTools();
      const allowedTools = this.filterTools(
        availableTools,
        request.allowedTools,
        request.disallowedTools,
      );

      // Build messages
      const messages = this.buildMessages(request);

      // Make the streaming API call
      const streamParams: any = {
        model: this.options.model || "claude-3-5-sonnet-20241022",
        max_tokens: this.options.maxTokens || 4096,
        messages,
      };

      if (request.systemPrompt || this.options.systemPrompt) {
        streamParams.system = request.systemPrompt || this.options.systemPrompt;
      }

      if (allowedTools.length > 0) {
        streamParams.tools = allowedTools;
      }

      const stream = await this.client.messages.stream(streamParams);

      for await (const chunk of stream) {
        if (
          chunk.type === "content_block_delta" &&
          chunk.delta.type === "text_delta"
        ) {
          yield chunk.delta.text;
        } else if (
          chunk.type === "content_block_start" &&
          chunk.content_block.type === "tool_use"
        ) {
          // Handle tool use in streaming
          const toolResult = await this.mcpClient.callTool(
            chunk.content_block.name,
            chunk.content_block.input,
          );
          yield `\n[Tool: ${chunk.content_block.name}]\n${toolResult}\n`;
        }
      }
    } catch (error) {
      throw new Error(
        `Claude API stream failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  /**
   * Build Claude API messages from request
   */
  private buildMessages(
    request: LLMRequest,
  ): Anthropic.Messages.MessageParam[] {
    const messages: Anthropic.Messages.MessageParam[] = [];

    // Debug logging
    console.log('🔍 Building messages with conversation history:', {
      hasConversationHistory: !!(request.conversationHistory && request.conversationHistory.length > 0),
      conversationHistoryLength: request.conversationHistory?.length || 0,
      conversationHistory: request.conversationHistory,
      hasUserInput: !!request.userInput,
      userInput: request.userInput,
      prompt: request.prompt
    });

    // Add conversation history if available
    if (request.conversationHistory && request.conversationHistory.length > 0) {
      for (const msg of request.conversationHistory) {
        messages.push({
          role: msg.role as "user" | "assistant",
          content: msg.content,
        });
      }
    }

    // Add current prompt (or user input if responding to a prompt)
    if (request.userInput) {
      messages.push({
        role: "user",
        content: request.userInput,
      });
    } else {
      messages.push({
        role: "user",
        content: request.prompt,
      });
    }

    console.log('📨 Final messages array:', messages);
    return messages;
  }

  /**
   * Filter tools based on allowed/disallowed lists
   */
  private filterTools(
    availableTools: Anthropic.Tool[],
    allowedTools?: string[],
    disallowedTools?: string[],
  ): Anthropic.Tool[] {
    let filteredTools = availableTools;

    if (allowedTools && allowedTools.length > 0) {
      filteredTools = filteredTools.filter(
        (tool) =>
          allowedTools.includes(tool.name) ||
          (tool.name.includes("_") &&
            allowedTools.includes(tool.name.split("_")[0])),
      );
    }

    if (disallowedTools && disallowedTools.length > 0) {
      filteredTools = filteredTools.filter(
        (tool) =>
          !disallowedTools.includes(tool.name) &&
          !(
            tool.name.includes("_") &&
            disallowedTools.includes(tool.name.split("_")[0])
          ),
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
   * Parse control signals from AI response content
   */
  private parseControlSignals(content: string): {
    isFinished?: boolean;
    needsUserInput?: boolean;
    userPrompt?: string;
  } {
    const signals: any = {};

    // Look for special markers in the content
    // Format: [CONTROL:FINISHED] or [CONTROL:USER_INPUT:prompt text]
    const controlRegex = /\[CONTROL:(.*?)\]/g;
    let match;

    while ((match = controlRegex.exec(content)) !== null) {
      const controlCommand = match[1];

      if (controlCommand && controlCommand === "FINISHED") {
        signals.isFinished = true;
      } else if (controlCommand && controlCommand.startsWith("USER_INPUT:")) {
        signals.needsUserInput = true;
        signals.userPrompt = controlCommand.substring("USER_INPUT:".length);
      }
    }

    // Default to NOT finished if no control signals found - let AI continue working
    // Only mark as finished if AI explicitly says so
    if (!signals.hasOwnProperty("isFinished") && !signals.needsUserInput) {
      signals.isFinished = false;
    }

    return signals;
  }

  /**
   * Cleanup resources
   */
  async cleanup(): Promise<void> {
    await this.mcpClient.cleanup();
  }
}


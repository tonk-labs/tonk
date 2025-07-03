import { query } from "@anthropic-ai/claude-code";

/**
 * Claude Code native request interface
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
 * Claude Code native response interface
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
 * MCP Server configuration
 */
export interface MCPServerConfig {
  [serverName: string]: {
    command: string;
    args: string[];
    env?: Record<string, string>;
  };
}

/**
 * Claude Code configuration options
 */
export interface ClaudeCodeOptions {
  systemPrompt?: string;
  maxTurns?: number;
  allowedTools?: string[];
  disallowedTools?: string[];
  mcpConfig?: MCPServerConfig;
  permissionMode?: "default" | "acceptEdits" | "bypassPermissions" | "plan";
  workingDirectory?: string;
  verbose?: boolean;
  outputFormat?: "text" | "json" | "stream-json";
}

/**
 * Claude Code provider implementation
 */
export class ClaudeCodeProvider {
  name = "claude-code";

  constructor(private options: ClaudeCodeOptions = {}) {}

  /**
   * Check if the provider is configured and ready to use
   */
  isConfigured(): boolean {
    return true;
  }

  async complete(request: LLMRequest): Promise<LLMResponse> {
    let finalResult = "";
    let sessionId: string | undefined;
    let totalCostUsd: number | undefined;

    const queryOptions = this.buildQueryOptions(request);

    console.log("🔍 Claude Code query options:", JSON.stringify(queryOptions, null, 2));
    console.log("🔍 Claude Code prompt:", request.prompt);

    try {
      for await (const message of query({
        prompt: request.prompt,
        abortController: request.abortController || new AbortController(),
        options: queryOptions,
      })) {
        console.log("📨 Claude Code message:", JSON.stringify(message, null, 2));
        // Extract text content from assistant messages
        if (message.type === "assistant" && message.message?.content) {
          for (const block of message.message.content) {
            if (block.type === "text") {
              finalResult += block.text;
            }
          }
        } else if (message.type === "result" && message.subtype === "success") {
          finalResult = message.result || finalResult;
          totalCostUsd = message.total_cost_usd;
          sessionId = message.session_id;
        }
      }

      return {
        content: finalResult,
        ...(totalCostUsd !== undefined && { totalCostUsd }),
        ...(sessionId !== undefined && { sessionId }),
        provider: this.name,
        success: true,
      };
    } catch (error) {
      console.error("❌ Claude Code query error:", error);
      if (error instanceof Error) {
        console.error("❌ Error stack:", error.stack);
      }
      return {
        content: "",
        ...(totalCostUsd !== undefined && { totalCostUsd }),
        ...(sessionId !== undefined && { sessionId }),
        provider: this.name,
        success: false,
        error: `Claude Code query failed: ${error instanceof Error ? error.message : String(error)}`,
      };
    }
  }

  async *stream(request: LLMRequest): AsyncIterable<string> {
    const queryOptions = this.buildQueryOptions(request);

    try {
      for await (const message of query({
        prompt: request.prompt,
        abortController: request.abortController || new AbortController(),
        options: queryOptions,
      })) {
        // Stream text content from assistant messages
        if (message.type === "assistant" && message.message?.content) {
          for (const block of message.message.content) {
            if (block.type === "text") {
              yield block.text;
            }
          }
        }
      }
    } catch (error) {
      throw new Error(
        `Claude Code stream failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  /**
   * Build Claude Code query options from LLM request
   */
  private buildQueryOptions(request: LLMRequest): any {
    const options: any = {
      maxTurns: request.maxTurns || this.options.maxTurns || 10,
      outputFormat:
        request.outputFormat || this.options.outputFormat || "stream-json",
      verbose: request.verbose ?? this.options.verbose ?? false,
    };

    if (request.systemPrompt || this.options.systemPrompt) {
      options.systemPrompt = request.systemPrompt || this.options.systemPrompt;
    }

    if (request.allowedTools || this.options.allowedTools) {
      options.allowedTools = request.allowedTools || this.options.allowedTools;
    }

    if (request.disallowedTools || this.options.disallowedTools) {
      options.disallowedTools =
        request.disallowedTools || this.options.disallowedTools;
    }

    if (request.mcpConfig || this.options.mcpConfig) {
      options.mcpConfig = request.mcpConfig || this.options.mcpConfig;
    }

    if (request.permissionMode || this.options.permissionMode) {
      options.permissionMode =
        request.permissionMode || this.options.permissionMode;
    }

    if (request.workingDirectory || this.options.workingDirectory) {
      options.cwd = request.workingDirectory || this.options.workingDirectory;
    }

    return options;
  }
}

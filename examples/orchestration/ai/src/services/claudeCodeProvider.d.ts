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
export declare class ClaudeCodeProvider {
    private options;
    name: string;
    constructor(options?: ClaudeCodeOptions);
    /**
     * Check if the provider is configured and ready to use
     */
    isConfigured(): boolean;
    complete(request: LLMRequest): Promise<LLMResponse>;
    stream(request: LLMRequest): AsyncIterable<string>;
    /**
     * Build Claude Code query options from LLM request
     */
    private buildQueryOptions;
}
//# sourceMappingURL=claudeCodeProvider.d.ts.map
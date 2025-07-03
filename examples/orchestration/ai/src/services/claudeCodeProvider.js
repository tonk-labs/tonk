import { query } from "@anthropic-ai/claude-code";
/**
 * Claude Code provider implementation
 */
export class ClaudeCodeProvider {
    options;
    name = "claude-code";
    constructor(options = {}) {
        this.options = options;
    }
    /**
     * Check if the provider is configured and ready to use
     */
    isConfigured() {
        return true;
    }
    async complete(request) {
        let finalResult = "";
        let sessionId;
        let totalCostUsd;
        const queryOptions = this.buildQueryOptions(request);
        try {
            for await (const message of query({
                prompt: request.prompt,
                abortController: request.abortController || new AbortController(),
                options: queryOptions,
            })) {
                // Extract text content from assistant messages
                if (message.type === "assistant" && message.message?.content) {
                    for (const block of message.message.content) {
                        if (block.type === "text") {
                            finalResult += block.text;
                        }
                    }
                }
                else if (message.type === "result" && message.subtype === "success") {
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
        }
        catch (error) {
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
    async *stream(request) {
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
        }
        catch (error) {
            throw new Error(`Claude Code stream failed: ${error instanceof Error ? error.message : String(error)}`);
        }
    }
    /**
     * Build Claude Code query options from LLM request
     */
    buildQueryOptions(request) {
        const options = {
            maxTurns: request.maxTurns || this.options.maxTurns || 10,
            outputFormat: request.outputFormat || this.options.outputFormat || "stream-json",
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
//# sourceMappingURL=claudeCodeProvider.js.map
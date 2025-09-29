import { generateText, streamText } from 'ai';
import { getVFSService } from '../../services/vfs-service';
import { getChatHistory, type ChatMessage } from './chat-history';
import {
  createOpenRouterProvider,
  MODEL,
  AGENT_SYSTEM_PROMPT,
  tonkTools
} from './code_agent';

// Type definitions for better type safety
interface ToolCall {
  toolCallId: string;
  toolName: string;
  args?: any;
  arguments?: any;
}

interface ToolResult {
  result?: any;
  output?: any;
}

interface FormattedToolCall {
  id: string;
  name: string;
  args: any;
  result: any;
}

export interface AgentServiceOptions {
  manifestUrl?: string;
  wsUrl?: string;
}

export class AgentService {
  private initialized = false;
  private vfs = getVFSService();
  private chatHistory: any = null; // Lazy load to avoid initialization issues
  private openrouter = createOpenRouterProvider();
  private abortController: AbortController | null = null;

  // Utility methods for consistent tool handling
  private getToolArgs(call: ToolCall): any {
    return 'args' in call ? call.args : call.arguments;
  }

  private getToolResult(result: ToolResult): any {
    return result && ('result' in result ? result.result : result.output);
  }

  private formatToolCalls(toolCalls: ToolCall[], toolResults: ToolResult[]): FormattedToolCall[] {
    return toolCalls.map((call, index) => ({
      id: call.toolCallId,
      name: call.toolName,
      args: this.getToolArgs(call),
      result: toolResults[index] ? this.getToolResult(toolResults[index]) : null,
    }));
  }

  private createAbortController(): AbortController {
    // Clean up existing controller first
    this.cleanupAbortController();
    this.abortController = new AbortController();
    return this.abortController;
  }

  private cleanupAbortController(): void {
    if (this.abortController) {
      this.abortController.abort();
      this.abortController = null;
    }
  }

  private async storeToolExecutions(toolCalls: ToolCall[], toolResults: ToolResult[]): Promise<void> {
    for (let i = 0; i < toolCalls.length; i++) {
      const call = toolCalls[i];
      const result = toolResults[i];
      const args = this.getToolArgs(call);
      const resultData = this.getToolResult(result);

      // Store the tool call as a hidden assistant message
      await this.chatHistory.addMessage({
        role: 'assistant',
        content: `Calling tool: ${call.toolName}\nArguments: ${JSON.stringify(args, null, 2)}`,
        hidden: true,
        toolCalls: [{
          id: call.toolCallId,
          name: call.toolName,
          args: args,
          result: null,
        }],
      });

      // Store the tool result as a tool message
      if (result) {
        await this.chatHistory.addMessage({
          role: 'tool',
          content: JSON.stringify(resultData, null, 2),
          hidden: true,
          toolName: call.toolName,
          toolCallId: call.toolCallId,
        });
      }
    }
  }

  private async* streamToolResults(toolCalls: ToolCall[], toolResults: ToolResult[], hasTextContent: boolean): AsyncGenerator<{ content: string; done: boolean; type?: 'text' | 'tool_call' | 'tool_result' }> {
    if (toolCalls.length === 0) return;

    // Add spacing if there was text before
    if (hasTextContent) {
      yield { content: '\n\n---\n\n', done: false, type: 'text' };
    }

    for (let i = 0; i < toolCalls.length; i++) {
      const call = toolCalls[i];
      const result = toolResults[i];

      // Show tool call
      const toolInfo = `🔧 **Tool called:** \`${call.toolName}\`\n`;
      yield { content: toolInfo, done: false, type: 'tool_call' };

      // Show simplified args (not full JSON for readability)
      const args = this.getToolArgs(call);
      if (args && Object.keys(args).length > 0) {
        const argsPreview = JSON.stringify(args, null, 2);
        const argsText = argsPreview.length > 300 ?
          argsPreview.substring(0, 300) + '...' :
          argsPreview;
        yield { content: `\`\`\`json\n${argsText}\n\`\`\`\n`, done: false, type: 'tool_call' };
      }

      // Show result if available
      if (result) {
        const resultData = this.getToolResult(result);
        if (resultData !== undefined && resultData !== null) {
          const resultStr = typeof resultData === 'string' ?
            resultData :
            JSON.stringify(resultData, null, 2);
          const resultPreview = resultStr.length > 200 ?
            '(result truncated)' :
            resultStr;
          yield { content: `✅ **Result:** ${resultPreview}\n\n`, done: false, type: 'tool_result' };
        }
      }
    }
  }

  async initialize(options: AgentServiceOptions = {}): Promise<void> {
    if (this.initialized) {
      return;
    }

    const manifestUrl = options.manifestUrl ||
      import.meta.env.VITE_TONK_MANIFEST_URL ||
      'http://localhost:8081/.manifest.tonk';

    const wsUrl = options.wsUrl ||
      import.meta.env.VITE_TONK_WS_URL ||
      'ws://localhost:8081';

    // Initialize VFS if not already initialized
    if (!this.vfs.isInitialized()) {
      await this.vfs.initialize(manifestUrl, wsUrl);
    }

    // Now get and initialize chat history after user service is ready
    this.chatHistory = getChatHistory();
    await this.chatHistory.initialize();

    this.initialized = true;
  }
//
  async sendMessage(prompt: string): Promise<ChatMessage> {
    if (!this.initialized || !this.chatHistory) {
      throw new Error('Agent service not initialized');
    }

    // Create abort controller for this request
    const abortController = this.createAbortController();

    try {
      // Add user message and get conversation history in single operation
      await this.chatHistory.addMessage({
        role: 'user',
        content: prompt,
      });
      const messages = this.chatHistory.formatForAI();

      console.log('[AgentService] Generating response with tools...');

      // Generate response with tools - stop when finish tool is called
      const result = await generateText({
        model: this.openrouter(MODEL),
        system: AGENT_SYSTEM_PROMPT,
        messages: messages,
        tools: tonkTools,
        maxRetries: 5,
        abortSignal: abortController.signal,
        stopWhen: ({ toolCalls }) => {
          return toolCalls?.some((call: any) => call.toolName === 'finish') ?? false;
        },
      });

      const { text } = result;
      const toolCalls = result.steps.flatMap(step => step.toolCalls || []);
      const toolResults = result.steps.flatMap(step => step.toolResults || []);

      console.log('[AgentService] Response generated:', {
        hasText: !!text,
        textLength: text?.length,
        toolCallsCount: toolCalls?.length || 0,
      });

      if (toolCalls.length > 0) {
        console.log('[AgentService] Tool calls made:', toolCalls.map(c => ({
          name: c.toolName,
          args: this.getToolArgs(c),
        })));

        console.log('[AgentService] Tool results:', toolResults.map((r, i) => ({
          toolName: toolCalls[i]?.toolName,
          result: this.getToolResult(r),
        })));

        // Check if finish tool was called
        const finishCall = toolCalls.find(c => c.toolName === 'finish');
        if (finishCall) {
          console.log('[AgentService] ✅ FINISH tool called - task complete!', this.getToolArgs(finishCall));
        }

        // Store tool executions using utility method
        await this.storeToolExecutions(toolCalls, toolResults);
      }

      // Format tool calls using utility method
      const formattedToolCalls = toolCalls.length > 0 ? this.formatToolCalls(toolCalls, toolResults) : undefined;

      // Add the final assistant message (visible)
      const assistantMessage = await this.chatHistory.addMessage({
        role: 'assistant',
        content: text || 'Task completed.',
        toolCalls: formattedToolCalls,
      });

      return assistantMessage;
    } catch (error) {
      console.error('Error generating response:', error);

      // Don't add error message if aborted
      if (error instanceof Error && error.name === 'AbortError') {
        console.log('[AgentService] Generation was aborted');
        throw error;
      }

      // Add error message to history
      await this.chatHistory.addMessage({
        role: 'assistant',
        content: `I encountered an error: ${error instanceof Error ? error.message : 'Unknown error'}`,
      });

      throw error;
    } finally {
      // Always clean up abort controller
      this.cleanupAbortController();
    }
  }

  async* streamMessage(prompt: string): AsyncGenerator<{ content: string; done: boolean; type?: 'text' | 'tool_call' | 'tool_result' }> {
    if (!this.initialized) {
      throw new Error('Agent service not initialized');
    }

    // Create new abort controller for this request
    const abortController = this.createAbortController();

    // Add user message and get conversation history in single operation
    await this.chatHistory.addMessage({
      role: 'user',
      content: prompt,
    });

    // Yield so chat will update correctly
    yield { content: '', done: false, type: 'text' };

    const messages = this.chatHistory.formatForAI();

    // Create the assistant message with streaming flag
    const assistantMessage = await this.chatHistory.addMessage({
      role: 'assistant',
      content: '',
      streaming: true,
    });

    try {
      console.log('[AgentService] Starting streaming response with tools...');

      // Stream response with tools - stop when finish tool is called
      const result = await streamText({
        model: this.openrouter(MODEL),
        system: AGENT_SYSTEM_PROMPT,
        messages: messages,
        tools: tonkTools,
        maxRetries: 5,
        abortSignal: abortController.signal,
        stopWhen: ({ toolCalls }) => {
          return toolCalls?.some((call: any) => call.toolName === 'finish') ?? false;
        },
      });

      let fullContent = '';

      // First, stream the text content
      for await (const chunk of result.textStream) {
        fullContent += chunk;
        // Update the streaming message in history immediately
        await this.chatHistory.updateStreamingMessage(assistantMessage.id, fullContent, true);
        yield { content: chunk, done: false, type: 'text' };
      }

      // Wait for the full response to complete
      const finalResult = await result;

      const steps = Array.isArray(finalResult.steps) ? finalResult.steps : [];

      console.log('[AgentService] Stream complete:', {
        hasText: !!fullContent,
        textLength: fullContent?.length,
        stepsCount: steps.length,
      });

      // Format tool calls for storage - extract from steps
      const toolCalls = steps.flatMap(step => step.toolCalls || []);
      const toolResults = steps.flatMap(step => step.toolResults || []);

      // Stream tool results using utility method
      yield* this.streamToolResults(toolCalls, toolResults, !!fullContent.trim());

      if (toolCalls.length > 0) {
        console.log('[AgentService] Tool calls made during stream:', toolCalls.map(c => ({
          name: c.toolName,
          args: this.getToolArgs(c),
        })));

        // Check if finish tool was called
        const finishCall = toolCalls.find(c => c.toolName === 'finish');
        if (finishCall) {
          console.log('[AgentService] ✅ FINISH tool called - task complete!', this.getToolArgs(finishCall));
        }

        // Store tool executions using utility method
        await this.storeToolExecutions(toolCalls, toolResults);
      }

      // Format tool calls using utility method
      const formattedToolCalls = toolCalls.length > 0 ? this.formatToolCalls(toolCalls, toolResults) : undefined;

      console.log('[AgentService] Marking streamed message as complete...');

      // Update the existing message to mark it as complete and add tool calls if any
      await this.chatHistory.updateStreamingMessage(assistantMessage.id, fullContent || 'Task completed.', false);

      // If there were tool calls, update the message with them
      if (formattedToolCalls) {
        const messageIndex = this.chatHistory.getMessages().findIndex(m => m.id === assistantMessage.id);
        if (messageIndex !== -1) {
          const messages = this.chatHistory.getMessages();
          messages[messageIndex].toolCalls = formattedToolCalls;
          // Force save to persist tool calls
          await this.chatHistory.saveHistory();
        }
      }

      console.log('[AgentService] Stream complete - message saved');

      yield { content: '', done: true, type: 'text' };
    } catch (error) {
      console.error('Error streaming response:', error);

      // Don't add error message if aborted
      if (error instanceof Error && error.name === 'AbortError') {
        console.log('[AgentService] Generation was aborted');
        await this.chatHistory.updateStreamingMessage(assistantMessage.id, fullContent || '', false);
        yield { content: '', done: true, type: 'text' };
        return;
      }

      const errorMsg = `I encountered an error: ${error instanceof Error ? error.message : 'Unknown error'}`;

      // Mark the streaming message as complete with error
      await this.chatHistory.updateStreamingMessage(assistantMessage.id, errorMsg, false);

      yield { content: errorMsg, done: true, type: 'text' };
      throw error;
    } finally {
      // Always clean up abort controller
      this.cleanupAbortController();
    }
  }

  getHistory(): ChatMessage[] {
    return this.chatHistory.getVisibleMessages();
  }

  getAllHistory(): ChatMessage[] {
    return this.chatHistory.getMessages();
  }

  async clearHistory(): Promise<void> {
    await this.chatHistory.clearHistory();
  }

  isInitialized(): boolean {
    return this.initialized;
  }

  getChatHistory(): any {
    return this.chatHistory;
  }

  // Cleanup method for proper resource management
  destroy(): void {
    console.log('[AgentService] Destroying service...');
    this.cleanupAbortController();
    this.initialized = false;
    this.chatHistory = null;
  }

  stopGeneration(): void {
    console.log('[AgentService] Stopping generation...');
    this.cleanupAbortController();
  }

  async updateMessage(messageId: string, newContent: string): Promise<void> {
    await this.chatHistory.updateMessage(messageId, newContent);
  }

  async deleteMessage(messageId: string): Promise<void> {
    await this.chatHistory.deleteMessage(messageId);
  }

  async deleteMessageAndFollowing(messageId: string): Promise<void> {
    await this.chatHistory.deleteMessagesFrom(messageId);
  }

  async* regenerateFrom(messageId: string, newContent?: string): AsyncGenerator<{ content: string; done: boolean; type?: 'text' | 'tool_call' | 'tool_result' }> {
    console.log('[AgentService] regenerateFrom called:', { messageId, newContent });

    // If new content provided, update the message
    if (newContent) {
      await this.chatHistory.updateMessage(messageId, newContent);
    }

    // Delete all messages after this one
    await this.chatHistory.deleteMessagesAfter(messageId);

    // Check what messages we have now
    const currentMessages = this.chatHistory.getMessages();
    console.log('[AgentService] Messages after deletion:', currentMessages.map(m => ({
      id: m.id,
      role: m.role,
      content: m.content.substring(0, 50) + '...'
    })));

    // Now generate a new response without adding the user message again
    if (!this.initialized) {
      throw new Error('Agent service not initialized');
    }

    // Create new abort controller for this request
    const abortController = this.createAbortController();

    // Get conversation history for context (already includes the edited message)
    const messages = this.chatHistory.formatForAI();

    console.log('[AgentService] Formatted messages for AI:', messages.length);

    // Create the assistant message with streaming flag
    const assistantMessage = await this.chatHistory.addMessage({
      role: 'assistant',
      content: '',
      streaming: true,
    });

    let fullContent = '';

    try {
      console.log('[AgentService] Regenerating response...');

      // Stream response with tools - stop when finish tool is called
      const result = await streamText({
        model: this.openrouter(MODEL),
        system: AGENT_SYSTEM_PROMPT,
        messages: messages,
        tools: tonkTools,
        maxRetries: 5,
        abortSignal: abortController.signal,
        stopWhen: ({ toolCalls }) => {
          return toolCalls?.some((call: any) => call.toolName === 'finish') ?? false;
        },
      });

      // Stream the text content
      for await (const chunk of result.textStream) {
        fullContent += chunk;
        // Update the streaming message in history immediately
        await this.chatHistory.updateStreamingMessage(assistantMessage.id, fullContent, true);
        yield { content: chunk, done: false, type: 'text' };
      }

      // Wait for the full response to complete
      const finalResult = await result;
      const steps = Array.isArray(finalResult.steps) ? finalResult.steps : [];

      // Format and show tool calls if any
      const toolCalls = steps.flatMap(step => step.toolCalls || []);
      const toolResults = steps.flatMap(step => step.toolResults || []);

      if (toolCalls.length > 0) {
        // Stream tool results using utility method
        yield* this.streamToolResults(toolCalls, toolResults, !!fullContent.trim());

        // Store tool executions using utility method
        await this.storeToolExecutions(toolCalls, toolResults);
      }

      // Format tool calls using utility method
      const formattedToolCalls = toolCalls.length > 0 ? this.formatToolCalls(toolCalls, toolResults) : undefined;

      // Update the existing message to mark it as complete
      await this.chatHistory.updateStreamingMessage(assistantMessage.id, fullContent || 'Task completed.', false);

      // If there were tool calls, update the message with them
      if (formattedToolCalls) {
        const messageIndex = this.chatHistory.getMessages().findIndex(m => m.id === assistantMessage.id);
        if (messageIndex !== -1) {
          const messages = this.chatHistory.getMessages();
          messages[messageIndex].toolCalls = formattedToolCalls;
          await this.chatHistory.saveHistory();
        }
      }

      yield { content: '', done: true, type: 'text' };
    } catch (error) {
      console.error('[AgentService] Error regenerating response:', error);

      // Don't add error message if aborted
      if (error instanceof Error && error.name === 'AbortError') {
        console.log('[AgentService] Generation was aborted');
        // Mark the message as complete even if aborted
        await this.chatHistory.updateStreamingMessage(assistantMessage.id, fullContent || '', false);
        yield { content: '', done: true, type: 'text' };
        return;
      }

      const errorMsg = `I encountered an error: ${error instanceof Error ? error.message : 'Unknown error'}`;
      // Update the streaming message with error
      await this.chatHistory.updateStreamingMessage(assistantMessage.id, errorMsg, false);

      yield { content: errorMsg, done: true, type: 'text' };
      throw error;
    } finally {
      // Always clean up abort controller
      this.cleanupAbortController();
    }
  }
}

// Singleton instance
let agentServiceInstance: AgentService | null = null;

export function getAgentService(): AgentService {
  if (!agentServiceInstance) {
    agentServiceInstance = new AgentService();
  }
  return agentServiceInstance;
}

export function resetAgentService(): void {
  if (agentServiceInstance) {
    agentServiceInstance.destroy();
  }
  agentServiceInstance = null;
}
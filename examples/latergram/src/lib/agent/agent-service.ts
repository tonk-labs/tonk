import { generateText, streamText } from 'ai';
import { getVFSService } from '../../services/vfs-service';
import { getChatHistory, type ChatMessage } from './chat-history';
import {
  createOpenRouterProvider,
  MODEL,
  AGENT_SYSTEM_PROMPT,
  tonkTools
} from './code_agent';

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

    // Add user message to history
    await this.chatHistory.addMessage({
      role: 'user',
      content: prompt,
    });

    // Get conversation history for context
    const messages = this.chatHistory.formatForAI();

    try {
      console.log('[AgentService] Generating response with tools...');

      // Generate response with tools - stop when finish tool is called
      const result = await generateText({
        model: this.openrouter(MODEL),
        system: AGENT_SYSTEM_PROMPT,
        messages: messages as any, // Type assertion needed due to AI SDK type limitations
        tools: tonkTools,
        maxRetries: 5,
        stopWhen: ({ toolCalls }: any) => {
          // Stop when the finish tool is called
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

      // Format tool calls for storage - ensure toolCalls is an array
      const calls = Array.isArray(toolCalls) ? toolCalls : [];
      const results = Array.isArray(toolResults) ? toolResults : [];

      console.log('[AgentService] Tool processing:', {
        callsCount: calls.length,
        resultsCount: results.length,
        hasText: !!text,
      });

      if (calls.length > 0) {
        console.log('[AgentService] Tool calls made:', calls.map(c => ({
          name: c.toolName,
          args: 'args' in c ? c.args : (c as any).arguments,
        })));

        console.log('[AgentService] Tool results:', results.map((r, i) => ({
          toolName: calls[i]?.toolName,
          result: 'result' in r ? r.result : r.output,
        })));

        // Check if finish tool was called
        const finishCall = calls.find(c => c.toolName === 'finish');
        if (finishCall) {
          console.log('[AgentService] ✅ FINISH tool called - task complete!', 'args' in finishCall ? finishCall.args : (finishCall as any).arguments);
        }
      }

      // Store tool calls and results as separate hidden messages for context
      if (calls.length > 0) {
        for (let i = 0; i < calls.length; i++) {
          const call = calls[i];
          const result = results[i];

          const args = 'args' in call ? call.args : (call as any).arguments;
          const resultData = result && ('result' in result ? result.result : result.output);

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

      const formattedToolCalls = calls.length > 0 ? calls.map((call, index) => ({
        id: call.toolCallId,
        name: call.toolName,
        args: 'args' in call ? call.args : (call as any).arguments,
        result: results[index] && ('result' in results[index] ? (results[index] as any).result : (results[index] as any).output),
      })) : undefined;

      // Add the final assistant message (visible)
      const assistantMessage = await this.chatHistory.addMessage({
        role: 'assistant',
        content: text || 'Task completed.',
        toolCalls: formattedToolCalls,
      });

      return assistantMessage;
    } catch (error) {
      console.error('Error generating response:', error);

      // Add error message to history
      await this.chatHistory.addMessage({
        role: 'assistant',
        content: `I encountered an error: ${error instanceof Error ? error.message : 'Unknown error'}`,
      });

      throw error;
    }
  }

  async* streamMessage(prompt: string): AsyncGenerator<{ content: string; done: boolean; type?: 'text' | 'tool_call' | 'tool_result' }> {
    if (!this.initialized) {
      throw new Error('Agent service not initialized');
    }

    // Create new abort controller for this request
    this.abortController = new AbortController();

    // Add user message to history
    await this.chatHistory.addMessage({
      role: 'user',
      content: prompt,
    });

    yield { content: '', done: true, type: 'text' };

    // Get conversation history for context
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
        messages: messages as any, // Type assertion needed due to AI SDK type limitations
        tools: tonkTools,
        maxRetries: 5,
        abortSignal: this.abortController.signal,
        stopWhen: ({ toolCalls }: any) => {
          // Stop when the finish tool is called
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

      // If there were tool calls, append them to the output
      if (toolCalls.length > 0) {
        // Add spacing if there was text before
        if (fullContent.trim()) {
          yield { content: '\n\n---\n\n', done: false, type: 'text' };
        }

        for (let i = 0; i < toolCalls.length; i++) {
          const call = toolCalls[i];
          const result = toolResults[i];

          // Show tool call
          const toolInfo = `🔧 **Tool called:** \`${call.toolName}\`\n`;
          yield { content: toolInfo, done: false, type: 'tool_call' };

          // Show simplified args (not full JSON for readability)
          const args = 'args' in call ? call.args : (call as any).arguments;
          if (args && Object.keys(args).length > 0) {
            const argsPreview = JSON.stringify(args, null, 2);
            const argsText = argsPreview.length > 300 ?
              argsPreview.substring(0, 300) + '...' :
              argsPreview;
            yield { content: `\`\`\`json\n${argsText}\n\`\`\`\n`, done: false, type: 'tool_call' };
          }

          // Show result if available
          if (result) {
            const resultData = 'result' in result ? result.result : result.output;
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

      if (toolCalls.length > 0) {
        console.log('[AgentService] Tool calls made during stream:', toolCalls.map(c => ({
          name: c.toolName,
          args: 'args' in c ? c.args : (c as any).arguments,
        })));

        // Check if finish tool was called
        const finishCall = toolCalls.find(c => c.toolName === 'finish');
        if (finishCall) {
          console.log('[AgentService] ✅ FINISH tool called - task complete!', 'args' in finishCall ? finishCall.args : (finishCall as any).arguments);
        }
      }

      // Store tool calls and results as separate hidden messages for context
      if (toolCalls.length > 0) {
        for (let i = 0; i < toolCalls.length; i++) {
          const call = toolCalls[i];
          const result = toolResults[i];

          const args = 'args' in call ? call.args : (call as any).arguments;
          const resultData = result && ('result' in result ? result.result : result.output);

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

      const formattedToolCalls = toolCalls.length > 0 ? toolCalls.map((call, index) => ({
        id: call.toolCallId,
        name: call.toolName,
        args: call.args,
        result: toolResults[index]?.result,
      })) : undefined;

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

      const errorMsg = `I encountered an error: ${error instanceof Error ? error.message : 'Unknown error'}`;

      // Mark the streaming message as complete with error
      await this.chatHistory.updateStreamingMessage(assistantMessage.id, errorMsg, false);

      yield { content: errorMsg, done: true, type: 'text' };
      throw error;
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

  stopGeneration(): void {
    if (this.abortController) {
      console.log('[AgentService] Stopping generation...');
      this.abortController.abort();
      this.abortController = null;
    }
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
    this.abortController = new AbortController();

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
        messages: messages as any,
        tools: tonkTools,
        maxRetries: 5,
        abortSignal: this.abortController.signal,
        stopWhen: ({ toolCalls }: any) => {
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
        if (fullContent.trim()) {
          yield { content: '\n\n---\n\n', done: false, type: 'text' };
        }

        for (let i = 0; i < toolCalls.length; i++) {
          const call = toolCalls[i];
          const result = toolResults[i];

          const toolInfo = `🔧 **Tool called:** \`${call.toolName}\`\n`;
          yield { content: toolInfo, done: false, type: 'tool_call' };

          const args = 'args' in call ? call.args : (call as any).arguments;
          if (args && Object.keys(args).length > 0) {
            const argsPreview = JSON.stringify(args, null, 2);
            const argsText = argsPreview.length > 300 ?
              argsPreview.substring(0, 300) + '...' :
              argsPreview;
            yield { content: `\`\`\`json\n${argsText}\n\`\`\`\n`, done: false, type: 'tool_call' };
          }

          if (result) {
            const resultData = 'result' in result ? result.result : result.output;
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

        // Store tool calls in history
        for (let i = 0; i < toolCalls.length; i++) {
          const call = toolCalls[i];
          const result = toolResults[i];
          const args = 'args' in call ? call.args : (call as any).arguments;
          const resultData = result && ('result' in result ? result.result : result.output);

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

      // Mark the message as complete
      const formattedToolCalls = toolCalls.length > 0 ? toolCalls.map((call, index) => ({
        id: call.toolCallId,
        name: call.toolName,
        args: 'args' in call ? call.args : (call as any).arguments,
        result: toolResults[index] && ('result' in toolResults[index] ? (toolResults[index] as any).result : (toolResults[index] as any).output),
      })) : undefined;

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
  agentServiceInstance = null;
}
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
  private chatHistory = getChatHistory();
  private openrouter = createOpenRouterProvider();

  async initialize(options: AgentServiceOptions = {}): Promise<void> {
    if (this.initialized) {
      return;
    }

    const manifestUrl = options.manifestUrl ||
      import.meta.env.VITE_TONK_MANIFEST_URL ||
      'http://localhost:6080/.manifest.tonk';

    const wsUrl = options.wsUrl ||
      import.meta.env.VITE_TONK_WS_URL ||
      'ws://localhost:6080';

    // Initialize VFS if not already initialized
    if (!this.vfs.isInitialized()) {
      await this.vfs.initialize(manifestUrl, wsUrl);
    }

    // Initialize chat history
    await this.chatHistory.initialize();

    this.initialized = true;
  }
//
  async sendMessage(prompt: string): Promise<ChatMessage> {
    if (!this.initialized) {
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

  async* streamMessage(prompt: string): AsyncGenerator<{ content: string; done: boolean }> {
    if (!this.initialized) {
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
      console.log('[AgentService] Starting streaming response with tools...');

      // Stream response with tools - stop when finish tool is called
      const result = await streamText({
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

      let fullContent = '';

      for await (const chunk of result.textStream) {
        fullContent += chunk;
        yield { content: chunk, done: false };
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

      console.log('[AgentService] Saving streamed message to history...');

      // Add complete assistant message to history (visible)
      await this.chatHistory.addMessage({
        role: 'assistant',
        content: fullContent || 'Task completed.',
        toolCalls: formattedToolCalls,
      });

      console.log('[AgentService] Stream complete - message saved');

      yield { content: '', done: true };
    } catch (error) {
      console.error('Error streaming response:', error);

      const errorMsg = `I encountered an error: ${error instanceof Error ? error.message : 'Unknown error'}`;

      // Add error message to history
      await this.chatHistory.addMessage({
        role: 'assistant',
        content: errorMsg,
      });

      yield { content: errorMsg, done: true };
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

const poop = ""
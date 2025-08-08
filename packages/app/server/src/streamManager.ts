import { createTonkAgent } from './mastra/agent.js';

interface ChatMessage {
  role: 'user' | 'assistant' | 'system';
  content: string;
}

interface ChatStreamRequest {
  message: string;
  conversationId: string;
  messageHistory?: ChatMessage[];
  maxSteps?: number;
  userName?: string;
}

interface StreamEvent {
  type: 'message_start' | 'content' | 'tool' | 'message_complete' | 'error';
  id: string;
  timestamp?: number;
  data?: string;
  tool?: string;
  status?: 'running' | 'complete' | 'error';
  error?: string;
}

export class UnifiedStreamManager {
  private currentMessageId: string = '';
  private agent: any = null;

  async initialize() {
    if (!this.agent) {
      this.agent = await createTonkAgent();
    }
  }

  async createUnifiedStream(request: ChatStreamRequest): Promise<ReadableStream<Uint8Array>> {
    await this.initialize();

    return new ReadableStream({
      start: (controller) => {
        this.processMessageStream(controller, request);
      }
    });
  }

  private async processMessageStream(
    controller: ReadableStreamDefaultController<Uint8Array>,
    request: ChatStreamRequest
  ) {
    this.currentMessageId = `msg_${Date.now()}_${Math.random().toString(36).substring(2, 11)}`;

    try {
      // Emit start event
      this.emitEvent(controller, {
        type: 'message_start',
        id: this.currentMessageId,
        timestamp: Date.now()
      });

      // Prepare messages for agent - include conversation history
      const messages = [
        ...(request.messageHistory || []).map(msg => ({
          role: msg.role as 'user' | 'assistant' | 'system',
          content: msg.content
        })),
        {
          role: 'user' as const,
          content: request.message
        }
      ];

      // Create runtime context
      const runtimeContext = new Map();
      if (request.userName) {
        runtimeContext.set('userName', request.userName);
      }

      // Create Mastra stream with tool orchestration
      const stream = await this.agent.stream(messages, {
        maxSteps: request.maxSteps || 25,
        runtimeContext,

        // Real-time tool execution feedback
        onStepFinish: ({ toolCalls, toolResults }: { toolCalls?: any[]; toolResults?: any[] }) => {
          toolCalls?.forEach((toolCall: any) => {
            this.emitEvent(controller, {
              type: 'tool',
              id: this.currentMessageId,
              tool: this.getFriendlyToolName(toolCall.toolName),
              status: 'running',
              timestamp: Date.now()
            });
          });

          toolResults?.forEach((result: any, index: number) => {
            const toolCall = toolCalls?.[index];
            if (toolCall) {
              this.emitEvent(controller, {
                type: 'tool',
                id: this.currentMessageId,
                tool: this.getFriendlyToolName(toolCall.toolName),
                status: result.success !== false ? 'complete' : 'error',
                timestamp: Date.now()
              });
            }
          });
        }
      });

      // Stream content in real-time
      for await (const chunk of stream.textStream) {
        this.emitEvent(controller, {
          type: 'content',
          id: this.currentMessageId,
          data: chunk,
          timestamp: Date.now()
        });
      }

      // Emit completion event
      this.emitEvent(controller, {
        type: 'message_complete',
        id: this.currentMessageId,
        timestamp: Date.now()
      });

    } catch (error) {
      console.error('Stream processing error:', error);

      this.emitEvent(controller, {
        type: 'error',
        id: this.currentMessageId,
        error: error instanceof Error ? error.message : 'Unknown error occurred',
        timestamp: Date.now()
      });
    } finally {
      controller.close();
    }
  }

  private emitEvent(
    controller: ReadableStreamDefaultController<Uint8Array>,
    event: StreamEvent
  ) {
    const eventLine = JSON.stringify(event) + '\n';
    const encoder = new TextEncoder();
    controller.enqueue(encoder.encode(eventLine));
  }

  private getFriendlyToolName(toolName: string): string {
    const friendlyNames: Record<string, string> = {
      'generate_widget': '🎨 Generating Widget',
      'write_widget_file': '📝 Writing File',
      'read_widget_file': '📖 Reading File',
      'read_widget_template': '📋 Reading Template',
      'read_base_widget': '🏗️ Reading Base Widget',
      'list_widgets': '📂 Listing Widgets',
      'read_widget_index': '📑 Reading Index',
    };

    return friendlyNames[toolName] || `🔧 ${toolName}`;
  }
}

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string;
  timestamp: number;
  toolCalls?: ToolCall[];
  toolName?: string;
  toolCallId?: string;
  hidden?: boolean;
}

export interface ToolCall {
  id: string;
  name: string;
  args: any;
  result?: any;
}
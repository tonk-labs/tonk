import { getVFSService } from '../../services/vfs-service';

const CHAT_HISTORY_PATH = '/etc/chat.json';
const MAX_MESSAGES = 10;

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string;
  timestamp: number;
  toolCalls?: Array<{
    id: string;
    name: string;
    args: any;
    result: any;
  }>;
  // For tool messages
  toolName?: string;
  toolCallId?: string;
  // Flag to hide from UI
  hidden?: boolean;
}

export interface ChatHistoryData {
  version: string;
  messages: ChatMessage[];
  lastUpdated: string;
}

export class ChatHistory {
  private messages: ChatMessage[] = [];
  private vfs = getVFSService();

  async initialize(): Promise<void> {
    if (!this.vfs.isInitialized()) {
      throw new Error('VFS service must be initialized before ChatHistory');
    }
    await this.loadHistory();
  }

  async loadHistory(): Promise<void> {
    try {
      const exists = await this.vfs.exists(CHAT_HISTORY_PATH);
      if (!exists) {
        await this.createEmptyHistory();
        return;
      }

      const content = await this.vfs.readFile(CHAT_HISTORY_PATH);
      const data: ChatHistoryData = JSON.parse(content);

      if (data.version === '1.0' && Array.isArray(data.messages)) {
        this.messages = data.messages;
      } else {
        console.warn('Invalid chat history format, creating new history');
        await this.createEmptyHistory();
      }
    } catch (error) {
      console.error('Failed to load chat history:', error);
      await this.createEmptyHistory();
    }
  }

  async saveHistory(): Promise<void> {
    const data: ChatHistoryData = {
      version: '1.0',
      messages: this.messages,
      lastUpdated: new Date().toISOString(),
    };

    try {
      const exists = await this.vfs.exists(CHAT_HISTORY_PATH);
      await this.vfs.writeFile(
        CHAT_HISTORY_PATH,
        JSON.stringify(data, null, 2),
        !exists
      );
    } catch (error) {
      console.error('Failed to save chat history:', error);
      throw error;
    }
  }

  async addMessage(message: Omit<ChatMessage, 'id' | 'timestamp'>): Promise<ChatMessage> {
    const newMessage: ChatMessage = {
      ...message,
      id: `msg_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`,
      timestamp: Date.now(),
    };

    this.messages.push(newMessage);

    // Trim to max messages
    if (this.messages.length > MAX_MESSAGES) {
      this.messages = this.messages.slice(-MAX_MESSAGES);
    }

    await this.saveHistory();
    return newMessage;
  }

  async clearHistory(): Promise<void> {
    this.messages = [];
    await this.saveHistory();
  }

  getMessages(): ChatMessage[] {
    return [...this.messages];
  }

  getLastMessages(count: number): ChatMessage[] {
    return this.messages.slice(-count);
  }

  private async createEmptyHistory(): Promise<void> {
    this.messages = [];
    await this.saveHistory();
  }

  // Helper to format messages for AI SDK - includes tool messages
  formatForAI(): Array<{ role: 'user' | 'assistant' | 'system' | 'tool'; content: string; toolCallId?: string }> {
    return this.messages.map(msg => {
      // For tool messages, include the toolCallId
      if (msg.role === 'tool') {
        return {
          role: msg.role,
          content: msg.content,
          toolCallId: msg.toolCallId,
        };
      }

      // For other messages, just pass them through
      return {
        role: msg.role as 'user' | 'assistant' | 'system',
        content: msg.content,
      };
    });
  }

  // Get messages for UI display (excludes hidden messages)
  getVisibleMessages(): ChatMessage[] {
    return this.messages.filter(msg => !msg.hidden);
  }
}

// Singleton instance
let chatHistoryInstance: ChatHistory | null = null;

export function getChatHistory(): ChatHistory {
  if (!chatHistoryInstance) {
    chatHistoryInstance = new ChatHistory();
  }
  return chatHistoryInstance;
}

export function resetChatHistory(): void {
  chatHistoryInstance = null;
}
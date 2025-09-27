import { getVFSService } from '../../services/vfs-service';
import { getUserService } from '../../services/user-service';

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
  private userService = getUserService();

  private getChatHistoryPath(): string {
    const userPath = this.userService.getChatHistoryPath();
    if (!userPath) {
      throw new Error(
        'User service not initialized - cannot determine chat history path'
      );
    }
    return userPath;
  }

  async initialize(): Promise<void> {
    if (!this.vfs.isInitialized()) {
      throw new Error('VFS service must be initialized before ChatHistory');
    }
    await this.loadHistory();
  }

  async loadHistory(): Promise<void> {
    try {
      const chatPath = this.getChatHistoryPath();
      const exists = await this.vfs.exists(chatPath);
      if (!exists) {
        await this.createEmptyHistory();
        return;
      }

      const content = await this.vfs.readFile(chatPath);
      const data: ChatHistoryData = content.content as any;

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
      const chatPath = this.getChatHistoryPath();
      const exists = await this.vfs.exists(chatPath);
      await this.vfs.writeFile(chatPath, { content: data as any }, !exists);
    } catch (error) {
      console.error('Failed to save chat history:', error);
      throw error;
    }
  }

  async addMessage(
    message: Omit<ChatMessage, 'id' | 'timestamp'>
  ): Promise<ChatMessage> {
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
  formatForAI(): Array<{
    role: 'user' | 'assistant' | 'system' | 'tool';
    content: string;
    toolCallId?: string;
  }> {
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

  async updateMessage(messageId: string, newContent: string): Promise<void> {
    const messageIndex = this.messages.findIndex(msg => msg.id === messageId);
    if (messageIndex !== -1) {
      this.messages[messageIndex].content = newContent;
      this.messages[messageIndex].timestamp = Date.now();
      await this.saveHistory();
    }
  }

  async deleteMessage(messageId: string): Promise<void> {
    const messageIndex = this.messages.findIndex(msg => msg.id === messageId);
    if (messageIndex !== -1) {
      this.messages.splice(messageIndex, 1);
      await this.saveHistory();
    }
  }

  async deleteMessagesAfter(messageId: string): Promise<void> {
    const messageIndex = this.messages.findIndex(msg => msg.id === messageId);
    if (messageIndex !== -1) {
      // Keep messages up to and including the one with messageId
      this.messages = this.messages.slice(0, messageIndex + 1);
      await this.saveHistory();
    }
  }

  async deleteMessagesFrom(messageId: string): Promise<void> {
    const messageIndex = this.messages.findIndex(msg => msg.id === messageId);
    if (messageIndex !== -1) {
      // Keep messages before the one with messageId (not including it)
      this.messages = this.messages.slice(0, messageIndex);
      await this.saveHistory();
    }
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

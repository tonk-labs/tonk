import type { ChatConfig, ChatMessage } from '../types';

interface ChatHistoryFile {
  version: number;
  messages: ChatMessage[];
  config: ChatConfig;
}

const CHAT_HISTORY_PATH = '/chat-history.json';

/**
 * Load chat history from VFS
 */
export async function loadChatHistory(): Promise<ChatHistoryFile | null> {
  try {
    // TODO: Replace with actual VFS read when available
    // For now, return null to initialize empty
    return null;
  } catch (error) {
    console.error('Failed to load chat history:', error);
    return null;
  }
}

/**
 * Save chat history to VFS
 */
export async function saveChatHistory(
  messages: ChatMessage[],
  config: ChatConfig
): Promise<void> {
  const data: ChatHistoryFile = {
    version: 1,
    messages,
    config,
  };

  try {
    // TODO: Replace with actual VFS write when available
    console.log('Saving chat history to VFS:', CHAT_HISTORY_PATH, data);
  } catch (error) {
    console.error('Failed to save chat history:', error);
    throw error;
  }
}

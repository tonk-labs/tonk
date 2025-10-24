import { useEffect } from 'react';
import { useChat } from '../stores/chatStore';
import { loadChatHistory, saveChatHistory } from '../utils/chatHistory';

/**
 * Hook to sync chat store with VFS and presence system
 */
export function useChatSync() {
  const { messages, config } = useChat();

  // Load chat history on mount
  useEffect(() => {
    loadChatHistory().then(data => {
      if (data) {
        // TODO: Hydrate store with loaded data
        console.log('Loaded chat history:', data);
      }
    });
  }, []);

  // Save to VFS whenever messages change
  useEffect(() => {
    if (messages.length > 0) {
      saveChatHistory(messages, config);
    }
  }, [messages, config]);

  // TODO: Add presence system broadcast/receive integration
}

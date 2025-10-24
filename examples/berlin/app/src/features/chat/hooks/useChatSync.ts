import { useEffect } from 'react';
import { useChat } from '../stores/chatStore';
import { loadChatHistory, saveChatHistory } from '../utils/chatHistory';

/**
 * Hook to sync chat store with VFS and presence system
 */
export function useChatSync() {
  const { messages, config, hydrateMessages } = useChat();

  // Load chat history on mount
  useEffect(() => {
    loadChatHistory()
      .then(data => {
        if (data) {
          hydrateMessages(data.messages, data.config);
        }
      })
      .catch(error => {
        console.error('Failed to load chat history in hook:', error);
      });
  }, [hydrateMessages]);

  // Save to VFS whenever messages change
  useEffect(() => {
    if (messages.length > 0) {
      saveChatHistory(messages, config);
    }
  }, [messages, config]);

  // TODO: Add presence system broadcast/receive integration
}

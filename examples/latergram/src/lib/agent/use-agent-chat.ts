import { useEffect } from 'react';
import { useAgentChatStore } from './agent-chat-store';

/**
 * Clean hook for agent chat functionality.
 * Handles initialization and provides a clean interface to the agent chat store.
 */
export function useAgentChat() {
  const {
    // State
    messages,
    isLoading,
    error,
    isReady,
    streamingContent,
    editingMessageId,
    editContent,

    // Actions
    initialize,
    sendMessage,
    clearConversation,
    stopGeneration,
    updateMessage,
    deleteMessage,
    setEditingMessage,
  } = useAgentChatStore();

  // Initialize on mount
  useEffect(() => {
    initialize();
  }, [initialize]);

  return {
    // State
    messages,
    isLoading,
    error,
    isReady,
    streamingContent,
    editingMessageId,
    editContent,

    // Actions
    sendMessage,
    clearConversation,
    stopGeneration,
    updateMessage,
    deleteMessage,
    setEditingMessage,
  };
}

// Export type for consumers
export type UseAgentChatReturn = ReturnType<typeof useAgentChat>;
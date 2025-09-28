import { useEffect, useRef } from 'react';
import { useChatStore } from '../../stores/chat-store';
import { getAgentService } from './agent-service';

export interface UseAgentStoreReturn {
  messages: any[];
  isLoading: boolean;
  error: string | null;
  isReady: boolean;
  sendMessage: (text: string) => Promise<void>;
  clearConversation: () => Promise<void>;
  stopGeneration: () => void;
  updateMessage: (messageId: string, newContent: string) => Promise<void>;
  deleteMessage: (messageId: string) => Promise<void>;
  editingMessageId: string | null;
  editContent: string;
  setEditingMessage: (id: string | null, content?: string) => void;
}

export function useAgentStore(): UseAgentStoreReturn {
  const {
    messages,
    isLoading,
    error,
    isReady,
    sendMessage,
    clearConversation,
    stopGeneration,
    updateMessage,
    deleteMessage,
    syncWithService,
    setReady,
    setError,
    editingMessageId,
    editContent,
    setEditingMessage,
  } = useChatStore();

  const agentService = useRef(getAgentService());
  const initializingRef = useRef(false);
  const syncIntervalRef = useRef<NodeJS.Timeout | undefined>(undefined);

  // Initialize agent service with delay to avoid race condition
  useEffect(() => {
    const initializeAgent = async () => {
      if (initializingRef.current) {
        return;
      }

      initializingRef.current = true;

      try {
        // Check if already initialized
        if (agentService.current.isInitialized()) {
          syncWithService();

          // Check for incomplete streaming messages on remount
          const messages = agentService.current.getHistory();
          const streamingMessage = messages.find(msg => msg.streaming === true);
          if (streamingMessage) {
            console.log('[useAgentStore] Found incomplete streaming message on remount:', streamingMessage.id);
            // Set loading state if there's an incomplete message
            // The content is already persisted, so it will be displayed
            // We just mark it as not streaming anymore since the stream was interrupted
            const chatHistory = agentService.current.getChatHistory();
            if (chatHistory) {
              await chatHistory.updateStreamingMessage(streamingMessage.id, streamingMessage.content, false);
              syncWithService();
            }
          }

          return;
        }

        // Add a small delay to ensure VFS is ready
        await new Promise(resolve => setTimeout(resolve, 1000));

        // Try to initialize if not already done
        if (!agentService.current.isInitialized()) {
          await agentService.current.initialize();
        }

        syncWithService();

        // Check for incomplete streaming messages after initialization
        const messages = agentService.current.getHistory();
        const streamingMessage = messages.find(msg => msg.streaming === true);
        if (streamingMessage) {
          console.log('[useAgentStore] Found incomplete streaming message after init:', streamingMessage.id);
          // Mark incomplete streaming messages as complete
          const chatHistory = agentService.current.getChatHistory();
          if (chatHistory) {
            await chatHistory.updateStreamingMessage(streamingMessage.id, streamingMessage.content, false);
            syncWithService();
          }
        }
      } catch (err) {
        console.error('Failed to initialize agent:', err);
        setError(err instanceof Error ? err.message : 'Failed to initialize agent');
      } finally {
        initializingRef.current = false;
      }
    };

    initializeAgent();

    // // Set up periodic sync for background updates
    // syncIntervalRef.current = setInterval(() => {
    //   if (agentService.current.isInitialized() && !isLoading) {
    //     syncWithService();
    //   }
    // }, 5000); // Sync every 5 seconds

    return () => {
      if (syncIntervalRef.current) {
        clearInterval(syncIntervalRef.current);
      }
    };
  }, []);

  return {
    messages,
    isLoading,
    error,
    isReady,
    sendMessage,
    clearConversation,
    stopGeneration,
    updateMessage,
    deleteMessage,
    editingMessageId,
    editContent,
    setEditingMessage,
  };
}
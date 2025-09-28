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
  const syncIntervalRef = useRef<NodeJS.Timeout>();

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
          return;
        }

        // Add a small delay to ensure VFS is ready
        await new Promise(resolve => setTimeout(resolve, 1000));

        // Try to initialize if not already done
        if (!agentService.current.isInitialized()) {
          await agentService.current.initialize();
        }

        syncWithService();
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
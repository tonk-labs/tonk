import { create } from 'zustand';
import { type AgentService, getAgentService } from './agent-service';
import type { ChatMessage } from './chat-history';

export interface AgentChatState {
  // State
  messages: ChatMessage[];
  isLoading: boolean;
  error: string | null;
  isReady: boolean;
  streamingContent: string;
  editingMessageId: string | null;
  editContent: string;
}

export interface AgentChatActions {
  // Internal setters
  setMessages: (messages: ChatMessage[]) => void;
  setLoading: (loading: boolean) => void;
  setError: (error: string | null) => void;
  setReady: (ready: boolean) => void;
  setStreamingContent: (content: string) => void;
  setEditingMessage: (id: string | null, content?: string) => void;

  // Public actions
  initialize: () => Promise<void>;
  sendMessage: (text: string) => Promise<void>;
  clearConversation: () => Promise<void>;
  stopGeneration: () => void;
  updateMessage: (messageId: string, newContent: string) => Promise<void>;
  deleteMessage: (messageId: string) => Promise<void>;
  syncWithService: () => void;
}

export type AgentChatStore = AgentChatState & AgentChatActions;

// Keep service instance outside store for singleton pattern
let agentService: AgentService | null = null;
let initializationPromise: Promise<void> | null = null;

const getOrCreateAgentService = (): AgentService => {
  if (!agentService) {
    agentService = getAgentService();
  }
  return agentService;
};

export const useAgentChatStore = create<AgentChatStore>((set, get) => ({
  // Initial state
  messages: [],
  isLoading: false,
  error: null,
  isReady: false,
  streamingContent: '',
  editingMessageId: null,
  editContent: '',

  // Internal setters
  setMessages: messages => set({ messages }),
  setLoading: loading => set({ isLoading: loading }),
  setError: error => set({ error }),
  setReady: ready => set({ isReady: ready }),
  setStreamingContent: content => set({ streamingContent: content }),
  setEditingMessage: (id, content = '') =>
    set({
      editingMessageId: id,
      editContent: content,
    }),

  // Initialize the agent service
  initialize: async () => {
    // Prevent multiple initializations
    if (initializationPromise) {
      return initializationPromise;
    }

    const service = getOrCreateAgentService();

    // Check if already initialized
    if (service.isInitialized()) {
      const history = service.getHistory();
      set({ messages: history, isReady: true });

      // Check for incomplete streaming messages
      const streamingMessage = history.find(msg => msg.streaming === true);
      if (streamingMessage) {
        console.log(
          '[AgentChatStore] Found incomplete streaming message:',
          streamingMessage.id
        );
        const chatHistory = service.getChatHistory();
        if (chatHistory) {
          await chatHistory.updateStreamingMessage(
            streamingMessage.id,
            streamingMessage.content,
            false
          );
          const updatedHistory = service.getHistory();
          set({ messages: updatedHistory });
        }
      }
      return;
    }

    initializationPromise = (async () => {
      try {
        // Add delay to ensure VFS is ready
        await new Promise(resolve => setTimeout(resolve, 1000));

        if (!service.isInitialized()) {
          await service.initialize();
        }

        const history = service.getHistory();
        set({ messages: history, isReady: true });

        // Handle incomplete streaming messages
        const streamingMessage = history.find(msg => msg.streaming === true);
        if (streamingMessage) {
          console.log(
            '[AgentChatStore] Found incomplete streaming message after init:',
            streamingMessage.id
          );
          const chatHistory = service.getChatHistory();
          if (chatHistory) {
            await chatHistory.updateStreamingMessage(
              streamingMessage.id,
              streamingMessage.content,
              false
            );
            const updatedHistory = service.getHistory();
            set({ messages: updatedHistory });
          }
        }
      } catch (err) {
        console.error('Failed to initialize agent:', err);
        set({
          error:
            err instanceof Error ? err.message : 'Failed to initialize agent',
        });
      } finally {
        initializationPromise = null;
      }
    })();

    return initializationPromise;
  },

  // Sync with service
  syncWithService: () => {
    const service = getOrCreateAgentService();
    if (service.isInitialized()) {
      const history = service.getHistory();
      set({ messages: history, isReady: true });
    }
  },

  // Send a message
  sendMessage: async text => {
    const { isReady, isLoading } = get();
    if (!text.trim() || !isReady || isLoading) {
      return;
    }

    const service = getOrCreateAgentService();
    set({ isLoading: true, error: null, streamingContent: '' });

    try {
      let accumulatedContent = '';

      // Stream the response
      for await (const chunk of service.streamMessage(text)) {
        if (chunk.content) {
          accumulatedContent += chunk.content;
          set({ streamingContent: accumulatedContent });
        }

        // Sync with service to get updated messages including the streaming one
        const updatedHistory = service.getHistory();
        set({ messages: updatedHistory });

        if (chunk.done) {
          set({ streamingContent: '' });
        }
      }
    } catch (err) {
      console.error('Failed to send message:', err);
      set({
        error: err instanceof Error ? err.message : 'Failed to send message',
      });

      // Sync messages even on error
      const updatedHistory = service.getHistory();
      set({ messages: updatedHistory });
    } finally {
      set({ isLoading: false });
    }
  },

  // Clear conversation
  clearConversation: async () => {
    const { isReady } = get();
    if (!isReady) return;

    const service = getOrCreateAgentService();
    try {
      await service.clearHistory();
      set({ messages: [], error: null, streamingContent: '' });
    } catch (err) {
      console.error('Failed to clear conversation:', err);
      set({
        error:
          err instanceof Error ? err.message : 'Failed to clear conversation',
      });
    }
  },

  // Stop generation
  stopGeneration: () => {
    const service = getOrCreateAgentService();
    service.stopGeneration();
    set({ isLoading: false });
  },

  // Update and regenerate message
  updateMessage: async (messageId, newContent) => {
    const { isReady, isLoading } = get();
    if (!isReady || isLoading) return;

    const service = getOrCreateAgentService();
    set({ isLoading: true, error: null, streamingContent: '' });

    try {
      let accumulatedContent = '';

      // Regenerate from this message with new content
      for await (const chunk of service.regenerateFrom(messageId, newContent)) {
        if (chunk.content) {
          accumulatedContent += chunk.content;
          set({ streamingContent: accumulatedContent });
        }

        // Sync with service to get updated messages
        const updatedHistory = service.getHistory();
        set({ messages: updatedHistory });

        if (chunk.done) {
          set({ streamingContent: '' });
        }
      }

      // Reset editing state after successful update
      set({ editingMessageId: null, editContent: '' });
    } catch (err) {
      console.error('Failed to update and regenerate:', err);
      set({
        error:
          err instanceof Error
            ? err.message
            : 'Failed to update and regenerate',
      });

      // Sync messages even on error
      const updatedHistory = service.getHistory();
      set({ messages: updatedHistory });
    } finally {
      set({ isLoading: false });
    }
  },

  // Delete message and following
  deleteMessage: async messageId => {
    const { isReady } = get();
    if (!isReady) return;

    const service = getOrCreateAgentService();
    try {
      await service.deleteMessageAndFollowing(messageId);
      const updatedHistory = service.getHistory();
      set({ messages: updatedHistory });
    } catch (err) {
      console.error('Failed to delete message:', err);
      set({
        error: err instanceof Error ? err.message : 'Failed to delete message',
      });
    }
  },
}));

// Export store instance for direct access (e.g., from error handlers)
export const agentChatStore = useAgentChatStore;

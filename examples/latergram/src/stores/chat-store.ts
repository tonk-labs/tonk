import { create } from 'zustand';
import { getAgentService } from '../lib/agent/agent-service';
import type { ChatMessage } from '../lib/agent/chat-history';

interface ChatStore {
  messages: ChatMessage[];
  isLoading: boolean;
  error: string | null;
  isReady: boolean;
  streamingContent: string;
  editingMessageId: string | null;
  editContent: string;

  // Actions
  setMessages: (messages: ChatMessage[]) => void;
  setLoading: (loading: boolean) => void;
  setError: (error: string | null) => void;
  setReady: (ready: boolean) => void;
  setStreamingContent: (content: string) => void;
  setEditingMessage: (id: string | null, content?: string) => void;

  // Service actions
  sendMessage: (text: string) => Promise<void>;
  clearConversation: () => Promise<void>;
  stopGeneration: () => void;
  updateMessage: (messageId: string, newContent: string) => Promise<void>;
  deleteMessage: (messageId: string) => Promise<void>;

  // Sync with service
  syncWithService: () => void;
}

const agentService = getAgentService();
let abortController: AbortController | null = null;

export const useChatStore = create<ChatStore>((set, get) => ({
  messages: [],
  isLoading: false,
  error: null,
  isReady: false,
  streamingContent: '',
  editingMessageId: null,
  editContent: '',

  setMessages: (messages) => set({ messages }),
  setLoading: (loading) => set({ isLoading: loading }),
  setError: (error) => set({ error }),
  setReady: (ready) => set({ isReady: ready }),
  setStreamingContent: (content) => set({ streamingContent: content }),
  setEditingMessage: (id, content = '') => set({
    editingMessageId: id,
    editContent: content
  }),

  syncWithService: () => {
    if (agentService.isInitialized()) {
      const history = agentService.getHistory();
      set({ messages: history, isReady: true });
    }
  },

  sendMessage: async (text) => {
    const { isReady, isLoading } = get();
    if (!text.trim() || !isReady || isLoading) {
      return;
    }

    set({ isLoading: true, error: null, streamingContent: '' });

    // Optimistically add user message
    const tempUserMessage: ChatMessage = {
      id: `temp_${Date.now()}`,
      role: 'user',
      content: text,
      timestamp: Date.now(),
    };

    set(state => ({ messages: [...state.messages, tempUserMessage] }));

    try {
      // Create a temporary assistant message for streaming
      const tempAssistantMessage: ChatMessage = {
        id: `temp_assistant_${Date.now()}`,
        role: 'assistant',
        content: '',
        timestamp: Date.now(),
      };

      set(state => ({ messages: [...state.messages, tempAssistantMessage] }));

      let accumulatedContent = '';

      // Stream the response
      for await (const chunk of agentService.streamMessage(text)) {
        if (chunk.content) {
          accumulatedContent += chunk.content;
          set({ streamingContent: accumulatedContent });

          // Update the temporary assistant message
          set(state => {
            const newMessages = [...state.messages];
            const lastMessage = newMessages[newMessages.length - 1];
            if (lastMessage && lastMessage.id.startsWith('temp_assistant_')) {
              lastMessage.content = accumulatedContent;
            }
            return { messages: newMessages };
          });
        }

        if (chunk.done) {
          // Update with the final history from the service
          const updatedHistory = agentService.getHistory();
          set({ messages: updatedHistory, streamingContent: '' });
        }
      }
    } catch (err) {
      console.error('Failed to send message:', err);
      set({ error: err instanceof Error ? err.message : 'Failed to send message' });

      // Remove temporary messages on error
      set(state => ({
        messages: state.messages.filter(msg => !msg.id.startsWith('temp_'))
      }));
    } finally {
      set({ isLoading: false });
    }
  },

  clearConversation: async () => {
    const { isReady } = get();
    if (!isReady) return;

    try {
      await agentService.clearHistory();
      set({ messages: [], error: null, streamingContent: '' });
    } catch (err) {
      console.error('Failed to clear conversation:', err);
      set({ error: err instanceof Error ? err.message : 'Failed to clear conversation' });
    }
  },

  stopGeneration: () => {
    agentService.stopGeneration();
    set({ isLoading: false });
  },

  updateMessage: async (messageId, newContent) => {
    const { isReady, isLoading } = get();
    if (!isReady || isLoading) return;

    set({ isLoading: true, error: null, streamingContent: '' });

    try {
      // Create a temporary assistant message for streaming
      const tempAssistantMessage: ChatMessage = {
        id: `temp_assistant_${Date.now()}`,
        role: 'assistant',
        content: '',
        timestamp: Date.now(),
      };

      // Update messages to show we're regenerating
      const currentMessages = agentService.getHistory();
      set({ messages: [...currentMessages, tempAssistantMessage] });

      let accumulatedContent = '';

      // Regenerate from this message with new content
      for await (const chunk of agentService.regenerateFrom(messageId, newContent)) {
        if (chunk.content) {
          accumulatedContent += chunk.content;
          set({ streamingContent: accumulatedContent });

          // Update the temporary assistant message
          set(state => {
            const newMessages = [...state.messages];
            const lastMessage = newMessages[newMessages.length - 1];
            if (lastMessage && lastMessage.id.startsWith('temp_assistant_')) {
              lastMessage.content = accumulatedContent;
            }
            return { messages: newMessages };
          });
        }

        if (chunk.done) {
          // Update with the final history from the service
          const updatedHistory = agentService.getHistory();
          set({ messages: updatedHistory, streamingContent: '' });
        }
      }

      // Reset editing state after successful update
      set({ editingMessageId: null, editContent: '' });
    } catch (err) {
      console.error('Failed to update and regenerate:', err);
      set({ error: err instanceof Error ? err.message : 'Failed to update and regenerate' });

      // Remove temporary message on error
      set(state => ({
        messages: state.messages.filter(msg => !msg.id.startsWith('temp_'))
      }));
    } finally {
      set({ isLoading: false });
    }
  },

  deleteMessage: async (messageId) => {
    const { isReady } = get();
    if (!isReady) return;

    try {
      await agentService.deleteMessageAndFollowing(messageId);
      const updatedHistory = agentService.getHistory();
      set({ messages: updatedHistory });
    } catch (err) {
      console.error('Failed to delete message:', err);
      set({ error: err instanceof Error ? err.message : 'Failed to delete message' });
    }
  },
}));
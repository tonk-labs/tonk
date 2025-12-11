import { FHS } from '../../../lib/paths';
import { StoreBuilder } from '../../../lib/storeBuilder';
import type { ChatConfig, ChatMessage, WindowState } from '../types';

interface ChatState {
  messages: ChatMessage[];
  typingUsers: Set<string>;
  windowState: WindowState;
  config: ChatConfig;
}

const initialState: ChatState = {
  messages: [],
  typingUsers: new Set(),
  windowState: {
    isOpen: false,
    position: { x: 100, y: 100 },
    size: { width: 400, height: 600 },
  },
  config: {
    maxHistory: 400,
  },
};

export const chatStore = StoreBuilder(initialState, undefined, {
  path: FHS.getServicePath('chat', 'messages.json'),
});

export const useChatStore = chatStore.useStore;

const createChatActions = () => {
  const store = chatStore;

  return {
    /**
     * Add a message to the store (optimistic update)
     */
    addMessage: (message: ChatMessage) => {
      store.set((state) => {
        state.messages.push(message);

        // Enforce history limit
        const { maxHistory } = state.config;
        if (maxHistory !== -1 && state.messages.length > maxHistory) {
          // Use splice for immer compatibility - mutates draft instead of reassigning
          state.messages.splice(0, state.messages.length - maxHistory);
        }
      });
    },

    /**
     * Set typing status for a user
     */
    setUserTyping: (userId: string, isTyping: boolean) => {
      store.set((state) => {
        if (isTyping) {
          state.typingUsers.add(userId);
        } else {
          state.typingUsers.delete(userId);
        }
      });
    },

    /**
     * Toggle chat window open/close
     */
    toggleWindow: () => {
      store.set((state) => {
        state.windowState.isOpen = !state.windowState.isOpen;
      });
    },

    /**
     * Update window position
     */
    updateWindowPosition: (x: number, y: number) => {
      store.set((state) => {
        state.windowState.position = { x, y };
      });
    },

    /**
     * Add or remove reaction to a message
     */
    toggleReaction: (messageId: string, emoji: string, userId: string) => {
      store.set((state) => {
        const message = state.messages.find((m) => m.id === messageId);
        if (!message) return;

        const reaction = message.reactions.find((r) => r.emoji === emoji);

        if (reaction) {
          const userIndex = reaction.userIds.indexOf(userId);
          if (userIndex > -1) {
            // Remove user from reaction
            reaction.userIds.splice(userIndex, 1);
            // Remove reaction if no users left
            if (reaction.userIds.length === 0) {
              message.reactions = message.reactions.filter((r) => r.emoji !== emoji);
            }
          } else {
            // Add user to reaction
            reaction.userIds.push(userId);
          }
        } else {
          // Create new reaction
          message.reactions.push({ emoji, userIds: [userId] });
        }
      });
    },

    /**
     * Clear all messages
     */
    clearMessages: () => {
      store.set((state) => {
        state.messages = [];
      });
    },

    /**
     * Hydrate messages from loaded data (e.g., VFS)
     */
    hydrateMessages: (messages: ChatMessage[], config?: Partial<ChatConfig>) => {
      store.set((state) => {
        state.messages = messages;
        if (config) {
          state.config = { ...state.config, ...config };
        }
      });
    },
  };
};

export const useChat = chatStore.createFactory(createChatActions());

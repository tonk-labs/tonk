import { sync, DocumentId } from '@tonk/keepsync';
import { create } from 'zustand';

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
  isStreaming?: boolean;
}

export interface ChatSession {
  id: string;
  agentId: string;
  messages: ChatMessage[];
  isLoading: boolean;
}

interface ChatData {
  sessions: ChatSession[];
}

interface ChatState extends ChatData {
  // Session management
  createSession: (agentId: string) => string;
  getSession: (agentId: string) => ChatSession | undefined;

  // Message management
  addMessage: (
    agentId: string,
    message: Omit<ChatMessage, 'id' | 'timestamp'>
  ) => void;
  updateMessage: (agentId: string, messageId: string, content: string) => void;
  setMessageStreaming: (
    agentId: string,
    messageId: string,
    isStreaming: boolean
  ) => void;

  // Loading states
  setLoading: (agentId: string, isLoading: boolean) => void;

  // Utilities
  clearSession: (agentId: string) => void;
}

export const useChatStore = create<ChatState>(
  sync(
    (set, get) => ({
      sessions: [],

      createSession: (agentId: string) => {
        const existingSession = get().sessions.find(s => s.agentId === agentId);
        if (existingSession) {
          return existingSession.id;
        }

        const sessionId = crypto.randomUUID();
        const newSession: ChatSession = {
          id: sessionId,
          agentId,
          messages: [],
          isLoading: false,
        };

        set(state => ({
          sessions: [...state.sessions, newSession],
        }));

        return sessionId;
      },

      getSession: (agentId: string) => {
        return get().sessions.find(s => s.agentId === agentId);
      },

      addMessage: (
        agentId: string,
        message: Omit<ChatMessage, 'id' | 'timestamp'>
      ) => {
        const messageId = crypto.randomUUID();
        const fullMessage: ChatMessage = {
          ...message,
          id: messageId,
          timestamp: Date.now(),
        };

        set(state => ({
          sessions: state.sessions.map(session =>
            session.agentId === agentId
              ? { ...session, messages: [...session.messages, fullMessage] }
              : session
          ),
        }));
      },

      updateMessage: (agentId: string, messageId: string, content: string) => {
        set(state => ({
          sessions: state.sessions.map(session =>
            session.agentId === agentId
              ? {
                  ...session,
                  messages: session.messages.map(msg =>
                    msg.id === messageId ? { ...msg, content } : msg
                  ),
                }
              : session
          ),
        }));
      },

      setMessageStreaming: (
        agentId: string,
        messageId: string,
        isStreaming: boolean
      ) => {
        set(state => ({
          sessions: state.sessions.map(session =>
            session.agentId === agentId
              ? {
                  ...session,
                  messages: session.messages.map(msg =>
                    msg.id === messageId ? { ...msg, isStreaming } : msg
                  ),
                }
              : session
          ),
        }));
      },

      setLoading: (agentId: string, isLoading: boolean) => {
        set(state => ({
          sessions: state.sessions.map(session =>
            session.agentId === agentId ? { ...session, isLoading } : session
          ),
        }));
      },

      clearSession: (agentId: string) => {
        set(state => ({
          sessions: state.sessions.map(session =>
            session.agentId === agentId ? { ...session, messages: [] } : session
          ),
        }));
      },
    }),
    {
      docId: 'chat-sessions' as DocumentId,
    }
  )
);

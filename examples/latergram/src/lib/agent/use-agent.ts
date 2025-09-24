import { useEffect, useState, useCallback, useRef } from 'react';
import { getAgentService } from './agent-service';
import type { ChatMessage } from './chat-history';

export interface UseAgentReturn {
  messages: ChatMessage[];
  isLoading: boolean;
  error: string | null;
  isReady: boolean;
  sendMessage: (text: string) => Promise<void>;
  clearConversation: () => Promise<void>;
  streamingContent: string;
}

export function useAgent(): UseAgentReturn {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isReady, setIsReady] = useState(false);
  const [streamingContent, setStreamingContent] = useState('');

  const agentService = useRef(getAgentService());
  const initializingRef = useRef(false);

  // Initialize agent service with delay to avoid race condition
  useEffect(() => {
    const initializeAgent = async () => {
      if (initializingRef.current || agentService.current.isInitialized()) {
        return;
      }

      initializingRef.current = true;

      try {
        // Add a small delay to ensure VFS is ready
        await new Promise(resolve => setTimeout(resolve, 1000));

        await agentService.current.initialize();
        const history = agentService.current.getHistory();
        setMessages(history);
        setIsReady(true);
      } catch (err) {
        console.error('Failed to initialize agent:', err);
        setError(err instanceof Error ? err.message : 'Failed to initialize agent');
      } finally {
        initializingRef.current = false;
      }
    };

    initializeAgent();
  }, []);

  const sendMessage = useCallback(async (text: string) => {
    if (!text.trim() || !isReady || isLoading) {
      return;
    }

    setIsLoading(true);
    setError(null);
    setStreamingContent('');

    // Optimistically add user message
    const tempUserMessage: ChatMessage = {
      id: `temp_${Date.now()}`,
      role: 'user',
      content: text,
      timestamp: Date.now(),
    };

    setMessages(prev => [...prev, tempUserMessage]);

    try {
      // Create a temporary assistant message for streaming
      const tempAssistantMessage: ChatMessage = {
        id: `temp_assistant_${Date.now()}`,
        role: 'assistant',
        content: '',
        timestamp: Date.now(),
      };

      setMessages(prev => [...prev, tempAssistantMessage]);

      let accumulatedContent = '';

      console.log('[useAgent] Starting to stream response...');

      // Stream the response
      for await (const chunk of agentService.current.streamMessage(text)) {
        console.log('[useAgent] Received chunk:', {
          hasContent: !!chunk.content,
          contentLength: chunk.content?.length,
          done: chunk.done
        });

        if (chunk.content) {
          accumulatedContent += chunk.content;
          setStreamingContent(accumulatedContent);

          // Update the temporary assistant message
          setMessages(prev => {
            const newMessages = [...prev];
            const lastMessage = newMessages[newMessages.length - 1];
            if (lastMessage && lastMessage.id.startsWith('temp_assistant_')) {
              lastMessage.content = accumulatedContent;
            }
            return newMessages;
          });
        }

        if (chunk.done) {
          console.log('[useAgent] Stream done, updating with final history...');
          // Update with the final history from the service
          const updatedHistory = agentService.current.getHistory();
          console.log('[useAgent] Updated history:', {
            messageCount: updatedHistory.length,
            lastMessage: updatedHistory[updatedHistory.length - 1],
          });
          setMessages(updatedHistory);
          setStreamingContent('');
        }
      }

      console.log('[useAgent] Stream loop completed');
    } catch (err) {
      console.error('Failed to send message:', err);
      setError(err instanceof Error ? err.message : 'Failed to send message');

      // Remove temporary messages on error
      setMessages(prev => prev.filter(msg => !msg.id.startsWith('temp_')));
    } finally {
      setIsLoading(false);
    }
  }, [isReady, isLoading]);

  const clearConversation = useCallback(async () => {
    if (!isReady) {
      return;
    }

    try {
      await agentService.current.clearHistory();
      setMessages([]);
      setError(null);
      setStreamingContent('');
    } catch (err) {
      console.error('Failed to clear conversation:', err);
      setError(err instanceof Error ? err.message : 'Failed to clear conversation');
    }
  }, [isReady]);

  return {
    messages,
    isLoading,
    error,
    isReady,
    sendMessage,
    clearConversation,
    streamingContent,
  };
}
import React, { useRef, useCallback, useEffect } from 'react';
import { useChatStore } from '../stores/chatStore';
import { aiService } from '../services/aiService';
import ChatHistory from './ChatHistory';
import ChatInput from './ChatInput';

interface TonkAgentProps {
  id: string;
  x: number;
  y: number;
  onMove: (id: string, deltaX: number, deltaY: number) => void;
  selected?: boolean;
}

const TonkAgent: React.FC<TonkAgentProps> = ({
  id,
  x,
  y,
  onMove,
  selected = false,
}) => {
  const dragRef = useRef<{ isDragging: boolean; lastX: number; lastY: number }>(
    {
      isDragging: false,
      lastX: 0,
      lastY: 0,
    }
  );

  const {
    createSession,
    getSession,
    addMessage,
    updateMessage,
    setMessageStreaming,
    setLoading,
    sessions,
  } = useChatStore();

  // Initialize session if it doesn't exist
  useEffect(() => {
    const existingSession = getSession(id);
    if (!existingSession) {
      createSession(id);
    }
  }, [id, createSession, getSession]);

  // Get current session reactively from store
  const session = sessions.find(s => s.agentId === id);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest('.chat-content')) {
      return;
    }

    e.stopPropagation();
    dragRef.current = {
      isDragging: true,
      lastX: e.clientX,
      lastY: e.clientY,
    };
  }, []);

  const handleMouseMove = useCallback(
    (e: MouseEvent) => {
      if (!dragRef.current.isDragging) return;

      const deltaX = e.clientX - dragRef.current.lastX;
      const deltaY = e.clientY - dragRef.current.lastY;

      onMove(id, deltaX, deltaY);

      dragRef.current.lastX = e.clientX;
      dragRef.current.lastY = e.clientY;
    },
    [id, onMove]
  );

  const handleMouseUp = useCallback(() => {
    dragRef.current.isDragging = false;
  }, []);

  useEffect(() => {
    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [handleMouseMove, handleMouseUp]);

  const handleSendMessage = useCallback(
    async (message: string) => {
      if (!session) return;

      addMessage(id, {
        role: 'user',
        content: message,
      });

      setLoading(id, true);

      try {
        const messages = session.messages.concat([
          {
            id: 'temp',
            role: 'user' as const,
            content: message,
            timestamp: Date.now(),
          },
        ]);

        const conversationHistory = messages.map(msg => ({
          role: msg.role,
          content: msg.content,
        }));

        const response = await aiService.streamChat(conversationHistory);

        addMessage(id, {
          role: 'assistant',
          content: '',
          isStreaming: true,
        });

        const currentSession = getSession(id);
        const assistantMessage =
          currentSession?.messages[currentSession.messages.length - 1];

        if (assistantMessage) {
          let fullContent = '';

          for await (const chunk of response.textStream) {
            fullContent += chunk;
            updateMessage(id, assistantMessage.id, fullContent);
          }

          setMessageStreaming(id, assistantMessage.id, false);
        }
      } catch (error) {
        console.error('Error sending message:', error);
        addMessage(id, {
          role: 'assistant',
          content: 'Sorry, I encountered an error. Please try again.',
        });
      } finally {
        setLoading(id, false);
      }
    },
    [
      session,
      id,
      addMessage,
      setLoading,
      getSession,
      updateMessage,
      setMessageStreaming,
    ]
  );

  if (!session) {
    return null;
  }

  return (
    <div
      className={`absolute bg-white rounded-lg shadow-xl border select-none ${
        selected ? 'border-blue-500 border-2' : 'border-gray-200'
      }`}
      style={{
        left: x,
        top: y,
        transform: 'translate(-50%, -50%)',
        width: '320px',
        height: '400px',
      }}
    >
      <div
        className="bg-purple-500 text-white px-4 py-2 rounded-t-lg cursor-grab active:cursor-grabbing flex items-center justify-between"
        onMouseDown={handleMouseDown}
      >
        <span className="font-medium">Tonk Agent</span>
        <div className="w-2 h-2 bg-green-400 rounded-full"></div>
      </div>

      <div
        className="chat-content flex flex-col h-[calc(100%-40px)]"
        onWheel={e => e.stopPropagation()}
      >
        <ChatHistory messages={session.messages} />
        <ChatInput
          onSendMessage={handleSendMessage}
          disabled={session.isLoading}
          placeholder="Ask me anything..."
        />
      </div>
    </div>
  );
};

export default TonkAgent;

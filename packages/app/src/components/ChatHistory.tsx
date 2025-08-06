import React, { useEffect, useRef } from 'react';
import { ChatMessage as ChatMessageType } from '../stores/chatStore';
import ChatMessage from './ChatMessage';

interface ChatHistoryProps {
  messages: ChatMessageType[];
}

const ChatHistory: React.FC<ChatHistoryProps> = ({ messages }) => {
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  return (
    <div
      ref={scrollRef}
      className="flex-1 overflow-y-auto p-3 space-y-2 bg-gray-50"
    >
      {messages.length === 0 ? (
        <div className="flex items-center justify-center h-full text-gray-500">
          <div className="text-center">
            <div className="text-lg mb-2">👋</div>
            <div>Start a conversation!</div>
          </div>
        </div>
      ) : (
        messages.map(message => (
          <ChatMessage key={message.id} message={message} />
        ))
      )}
    </div>
  );
};

export default ChatHistory;

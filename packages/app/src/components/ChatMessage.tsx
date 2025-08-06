import React from 'react';
import { ChatMessage as ChatMessageType } from '../stores/chatStore';

interface ChatMessageProps {
  message: ChatMessageType;
}

const ChatMessage: React.FC<ChatMessageProps> = ({ message }) => {
  const isUser = message.role === 'user';

  return (
    <div className={`flex ${isUser ? 'justify-end' : 'justify-start'} mb-3`}>
      <div
        className={`max-w-[80%] px-3 py-2 rounded-lg ${
          isUser
            ? 'bg-blue-500 text-white rounded-br-sm'
            : 'bg-gray-200 text-gray-800 rounded-bl-sm'
        }`}
      >
        <div className="text-sm whitespace-pre-wrap break-words">
          {message.content}
          {message.isStreaming && (
            <span className="inline-block w-2 h-4 ml-1 bg-current animate-pulse" />
          )}
        </div>
        <div
          className={`text-xs mt-1 opacity-70 ${isUser ? 'text-blue-100' : 'text-gray-500'}`}
        >
          {new Date(message.timestamp).toLocaleTimeString([], {
            hour: '2-digit',
            minute: '2-digit',
          })}
        </div>
      </div>
    </div>
  );
};

export default ChatMessage;

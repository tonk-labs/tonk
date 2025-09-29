import React from 'react';
import { Bot, StopCircle } from 'lucide-react';

interface ChatLoadingDotsProps {
  onStop?: () => void;
}

const ChatLoadingDots: React.FC<ChatLoadingDotsProps> = ({ onStop }) => {
  return (
    <div className="flex justify-start">
      <div className="flex gap-3 max-w-[70%]">
        <div className="flex-shrink-0 w-6 h-6 rounded-full bg-gray-600 flex items-center justify-center">
          <Bot className="w-4 h-4 text-white" />
        </div>
        <div className="flex items-center gap-2">
          <div className="px-3 py-1.5 rounded-lg bg-white border border-gray-200">
            <div className="flex gap-1">
              <div className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce" />
              <div className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce delay-100" />
              <div className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce delay-200" />
            </div>
          </div>
          {onStop && (
            <button
              type="button"
              onClick={onStop}
              className="p-1.5 bg-red-500 text-white rounded-full hover:bg-red-600 transition-colors"
              title="Stop generation"
            >
              <StopCircle className="w-3 h-3" />
            </button>
          )}
        </div>
      </div>
    </div>
  );
};

export default ChatLoadingDots;
import React from 'react';
import { Bot, Trash2, Loader2 } from 'lucide-react';

interface ChatHeaderProps {
  isReady: boolean;
  messageCount: number;
  onClear: () => void;
}

const ChatHeader: React.FC<ChatHeaderProps> = ({
  isReady,
  messageCount,
  onClear,
}) => {
  const handleClearClick = () => {
    if (window.confirm('Are you sure you want to clear the conversation history?')) {
      onClear();
    }
  };

  return (
    <div className="bg-white border-b border-gray-200 px-4 py-2">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Bot className="w-4 h-4 text-blue-500" />
          <h1 className="text-xs font-semibold text-gray-800">Agent Chat</h1>
          {!isReady && (
            <span className="text-[10px] text-amber-600 flex items-center gap-1">
              <Loader2 className="w-3 h-3 animate-spin" />
              Initializing...
            </span>
          )}
          {isReady && (
            <span className="text-[10px] text-green-600">Ready</span>
          )}
        </div>
        <button
          type="button"
          onClick={handleClearClick}
          className="p-1 hover:bg-gray-100 rounded transition-colors"
          title="Clear conversation"
          disabled={!isReady || messageCount === 0}
        >
          <Trash2 className="w-3 h-3 text-gray-600" />
        </button>
      </div>
    </div>
  );
};

export default ChatHeader;
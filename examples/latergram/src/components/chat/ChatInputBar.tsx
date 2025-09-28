import React, { useState } from 'react';
import { Send, StopCircle, Trash } from 'lucide-react';
import ChatInput from './ChatInput';

interface ChatInputBarProps {
  onSendMessage: (text: string) => void;
  isLoading: boolean;
  isReady: boolean;
  inputRef: React.RefObject<HTMLTextAreaElement>;
  onClear: () => Promise<void>;
  onStop: () => void;
}

const ChatInputBar: React.FC<ChatInputBarProps> = ({
  onSendMessage,
  isLoading,
  isReady,
  inputRef,
  onClear,
  onStop,
}) => {
  const [prompt, setPrompt] = useState('');

  const handleSubmit = (e?: React.FormEvent) => {
    e?.preventDefault();
    // Allow typing but prevent sending while loading
    if (!prompt.trim() || !isReady || isLoading) return;
    onSendMessage(prompt.trim());
    setPrompt('');
  };

  const handleClear = async () => {
    if (window.confirm('Clear conversation?')) {
      await onClear();
    }
  };

  return (
    <div className="bg-white border-t border-gray-200 px-3 py-2">
      <form onSubmit={handleSubmit} className="max-w-3xl mx-auto">
        <div className="flex gap-2">
          <ChatInput
            inputRef={inputRef}
            value={prompt}
            onChange={setPrompt}
            onSubmit={handleSubmit}
            disabled={!isReady}
            placeholder={
              isReady
                ? 'Type a message... (Shift+Enter for new line)'
                : 'Waiting for initialization...'
            }
          />
          <div className="flex flex-row gap-2 group relative">
          {!isLoading && <button
            type="submit"
            disabled={!prompt.trim() || !isReady || isLoading}
            className="px-3 py-1.5 bg-blue-500 text-white rounded-full hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            <Send className="w-3 h-3" />
          </button>}
          {isLoading && <button
            type="submit"
            onClick={onStop}
            className="px-3 py-1.5 bg-blue-500 text-white rounded-full hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            <StopCircle className="w-3 h-3" />
          </button>}
          <button
            type="submit"
            onClick={handleClear}
            className="px-3 py-1.5 bg-red-500 text-white rounded-full h-full hover:bg-red-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors "
          >
            <Trash className="w-3 h-3" />
          </button>
          </div>
        </div>
      </form>
    </div>
  );
};

export default ChatInputBar;
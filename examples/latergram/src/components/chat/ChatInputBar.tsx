import React, { useState } from 'react';
import { Send } from 'lucide-react';
import ChatInput from './ChatInput';

interface ChatInputBarProps {
  onSendMessage: (text: string) => void;
  isLoading: boolean;
  isReady: boolean;
  inputRef: React.RefObject<HTMLTextAreaElement>;
}

const ChatInputBar: React.FC<ChatInputBarProps> = ({
  onSendMessage,
  isLoading,
  isReady,
  inputRef,
}) => {
  const [prompt, setPrompt] = useState('');

  const handleSubmit = (e?: React.FormEvent) => {
    e?.preventDefault();
    if (!prompt.trim() || !isReady || isLoading) return;
    onSendMessage(prompt.trim());
    setPrompt('');
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
            disabled={!isReady || isLoading}
            placeholder={
              isReady
                ? 'Type a message... (Shift+Enter for new line)'
                : 'Waiting for initialization...'
            }
          />
          <button
            type="submit"
            disabled={!prompt.trim() || !isReady || isLoading}
            className="px-3 py-1.5 bg-blue-500 text-white rounded-full hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            <Send className="w-3 h-3" />
          </button>
        </div>
      </form>
    </div>
  );
};

export default ChatInputBar;
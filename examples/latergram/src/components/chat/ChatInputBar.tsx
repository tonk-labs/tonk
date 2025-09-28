import React, { useCallback, useState } from 'react';
import { SendHorizonal, StopCircle, Trash } from 'lucide-react';
import { useKeyboardSubmit } from './hooks';

interface ChatInputBarProps {
  onSendMessage: (text: string) => void;
  isLoading: boolean;
  isReady: boolean;
  inputRef: React.RefObject<HTMLInputElement>;
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
  
  const handleSubmit = useCallback((e?: React.FormEvent) => {
    e?.preventDefault();
    // Allow typing but prevent sending while loading
    if (!prompt.trim() || !isReady || isLoading) return;
    onSendMessage(prompt.trim());
    setPrompt('');
  }, [prompt, isReady, isLoading, onSendMessage]);

  const { handleKeyDown } = useKeyboardSubmit(handleSubmit);

  const handleClear = async () => {
    if (window.confirm('Clear conversation?')) {
      await onClear();
    }
  };

  return (
    <div className="bg-white border-t border-gray-200 px-3 py-2">
      <form onSubmit={handleSubmit} className="max-w-3xl mx-auto">
        <div className="flex gap-2">
          <input
            ref={inputRef}
            value={prompt}
            onChange={e => setPrompt(e.target.value)}
            onKeyDown={handleKeyDown}
            disabled={!isReady}
            placeholder={
              isReady
                ? 'Type a message...'
                : 'Waiting for initialization...'
            }
            className="flex-1 px-3 py-1.5 border-0 border-gray-300 rounded-lg focus:outline-none text-sm disabled:bg-gray-100 ring-0 outline-none outline-0"
          />
          <div className="flex flex-row gap-2 group relative items-center justify-start">
            {!isLoading && (
              <button
                type="submit"
                disabled={!prompt.trim() || !isReady || isLoading}
                className="h-10 px-3 py-1.5 bg-blue-500 text-white rounded-full hover:bg-blue-600 disabled:bg-gray-500 disabled:cursor-not-allowed transition-colors"
              >
                <SendHorizonal className="w-4 h-4" />
              </button>
            )}
            {isLoading && (
              <button
                type="button"
                onClick={onStop}
                className="h-10 px-3 py-1.5 bg-blue-500 text-white rounded-full hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              >
                <StopCircle className="w-4 h-4" />
              </button>
            )}
            <button
              type="button"
              onClick={handleClear}
              className="h-10 px-3 py-1.5 bg-red-500 text-white rounded-full hover:bg-red-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors "
            >
              <Trash className="w-4 h-4" />
            </button>
          </div>
        </div>
      </form>
    </div>
  );
};

export default ChatInputBar;

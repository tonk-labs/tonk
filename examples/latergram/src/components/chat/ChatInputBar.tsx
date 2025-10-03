import { SendHorizonal, StopCircle, Trash } from 'lucide-react';
import type React from 'react';
import { useCallback, useState } from 'react';
import { useKeyboardSubmit } from './hooks';

interface ChatInputBarProps {
  onSendMessage: (
    text: string,
    context?: {
      currentView?: string;
      selectedFile?: string;
      fileType?: 'component' | 'store' | 'page' | 'generic';
    }
  ) => void;
  isLoading: boolean;
  isReady: boolean;
  inputRef: React.RefObject<HTMLTextAreaElement>;
  onClear: () => Promise<void>;
  onStop: () => void;
  context?: {
    currentView?: string;
    selectedFile?: string;
    fileType?: 'component' | 'store' | 'page' | 'generic';
  };
}

const ChatInputBar: React.FC<ChatInputBarProps> = ({
  onSendMessage,
  isLoading,
  isReady,
  inputRef,
  onClear,
  onStop,
  context,
}) => {
  const [prompt, setPrompt] = useState('');

  const handleSubmit = useCallback(
    (e?: React.FormEvent) => {
      e?.preventDefault();
      // Allow typing but prevent sending while loading
      if (!prompt.trim() || !isReady || isLoading) return;
      onSendMessage(prompt.trim(), context);
      setPrompt('');
    },
    [prompt, isReady, isLoading, onSendMessage, context]
  );

  const { handleKeyDown } = useKeyboardSubmit(handleSubmit);

  const handleClear = async () => {
    if (window.confirm('Clear conversation?')) {
      await onClear();
    }
  };

  return (
    <div className="bg-white border-t border-gray-200 px-3 py-2 safe-area-bottom">
      <form onSubmit={handleSubmit} className="max-w-3xl mx-auto">
        <div className="flex gap-2 items-center justify-center">
          <textarea
            style={{
              resize: 'none',
              fontSize: 'max(16px, 1rem)',
            }}
            ref={inputRef}
            value={prompt}
            onChange={e => setPrompt(e.target.value)}
            onKeyDown={handleKeyDown}
            onFocus={e => {
              setTimeout(() => {
                e.target.scrollIntoView({
                  behavior: 'smooth',
                  block: 'center',
                });
              }, 300);
            }}
            disabled={!isReady}
            placeholder={
              isReady ? 'Type a message...' : 'Waiting for initialization...'
            }
            className="flex-1 px-3 py-1.5 border-0 border-gray-300 rounded-lg focus:outline-none disabled:bg-gray-100 ring-0 outline-none outline-0"
            rows={1}
            inputMode="text"
          />
          <div className="flex flex-row gap-2 group relative items-center justify-start">
            {!isLoading && (
              <button
                title="Send Message"
                type="submit"
                disabled={!prompt.trim() || !isReady || isLoading}
                className="h-10 px-3 py-1.5 bg-blue-500 text-white rounded-full hover:bg-blue-600 disabled:bg-gray-500 disabled:cursor-not-allowed transition-colors"
              >
                <SendHorizonal className="w-4 h-4" />
              </button>
            )}
            {isLoading && (
              <button
                title="Stop Agent"
                type="button"
                onClick={onStop}
                className="h-10 px-3 py-1.5 bg-red-500 text-white rounded-full hover:bg-red-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              >
                <StopCircle className="w-4 h-4" />
              </button>
            )}
            <button
              title="Clear Conversation"
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

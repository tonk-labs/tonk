import React from 'react';
import { useKeyboardSubmit } from './hooks';

interface ChatInputProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  disabled?: boolean;
  placeholder?: string;
  rows?: number;
  inputRef?: React.RefObject<HTMLTextAreaElement>;
  isLoading?: boolean;
}

const ChatInput: React.FC<ChatInputProps> = ({
  value,
  onChange,
  onSubmit,
  disabled = false,
  placeholder = 'Type a message... (Shift+Enter for new line)',
  rows = 1,
  inputRef
}) => {
  const { handleKeyDown } = useKeyboardSubmit(onSubmit);

  return (
    <textarea
      ref={inputRef}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      onKeyDown={handleKeyDown}
      placeholder={placeholder}
      className="flex-1 px-3 py-1.5 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 text-xs disabled:bg-gray-100 resize-none"
      disabled={disabled}
      rows={rows}
    />
  );
};

export default ChatInput;
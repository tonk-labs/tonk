import React from 'react';
import { AlertCircle } from 'lucide-react';

interface ChatErrorBarProps {
  error: string;
}

const ChatErrorBar: React.FC<ChatErrorBarProps> = ({ error }) => {
  return (
    <div className="bg-red-50 border-b border-red-200 px-4 py-2">
      <div className="flex items-center gap-2 text-xs text-red-700">
        <AlertCircle className="w-3 h-3" />
        {error}
      </div>
    </div>
  );
};

export default ChatErrorBar;
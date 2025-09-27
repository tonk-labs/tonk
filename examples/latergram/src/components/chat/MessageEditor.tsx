import React from 'react';
import { Check, X, AlertCircle } from 'lucide-react';
import { useKeyboardSubmit } from './hooks';

interface MessageEditorProps {
  initialContent: string;
  onSave: () => void;
  onCancel: () => void;
  onChange: (content: string) => void;
  showWarning?: boolean;
  warningMessage?: string;
}

const MessageEditor: React.FC<MessageEditorProps> = ({
  initialContent,
  onSave,
  onCancel,
  onChange,
  showWarning = false,
  warningMessage = 'Editing will delete all messages after this one and regenerate the response',
}) => {
  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      onSave();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      onCancel();
    }
  };

  return (
    <div className="space-y-2">
      {showWarning && (
        <div className="text-[10px] text-amber-600 bg-amber-50 p-1.5 rounded flex items-start gap-1">
          <AlertCircle className="w-3 h-3 flex-shrink-0 mt-0.5" />
          <span>{warningMessage}</span>
        </div>
      )}
      <textarea
        value={initialContent}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={handleKeyDown}
        className="w-full p-2 border border-gray-300 rounded text-xs text-black"
        rows={3}
        placeholder="Press Enter to send, Shift+Enter for new line, Esc to cancel"
        autoFocus
      />
      <div className="flex gap-2 items-center">
        <button
          type="button"
          onClick={onSave}
          className="px-2 py-1 bg-green-500 text-white rounded text-xs hover:bg-green-600 flex items-center gap-1"
          title="Save edit and regenerate response (will delete all following messages)"
        >
          <Check className="w-3 h-3" />
          <span>Confirm Edit</span>
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="px-2 py-1 bg-gray-500 text-white rounded text-xs hover:bg-gray-600 flex items-center gap-1"
          title="Cancel (Esc)"
        >
          <X className="w-3 h-3" />
          <span>Cancel</span>
        </button>
        <span className="text-[10px] text-gray-500 ml-2">
          Enter to send • Esc to cancel
        </span>
      </div>
    </div>
  );
};

export default MessageEditor;
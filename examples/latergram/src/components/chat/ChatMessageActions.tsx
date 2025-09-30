import { Edit2, Trash2 } from 'lucide-react';
import type React from 'react';

interface ChatMessageActionsProps {
  messageId: string;
  onEdit: () => void;
  onDelete: () => void;
  isUser: boolean;
}

const ChatMessageActions: React.FC<ChatMessageActionsProps> = ({
  messageId,
  onEdit,
  onDelete,
  isUser,
}) => {
  if (!isUser) return null;

  return (
    <div className="opacity-0 group-hover:opacity-100 transition-opacity flex gap-1">
      <button
        type="button"
        onClick={onEdit}
        className="p-0.5 hover:bg-blue-600 rounded"
        title="Edit and regenerate from here"
      >
        <Edit2 className="w-3 h-3" />
      </button>
      <button
        type="button"
        onClick={onDelete}
        className="p-0.5 hover:bg-red-600 rounded"
        title="Delete from here"
      >
        <Trash2 className="w-3 h-3" />
      </button>
    </div>
  );
};

export default ChatMessageActions;

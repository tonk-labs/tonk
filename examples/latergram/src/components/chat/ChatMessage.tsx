import React from 'react';
import { Bot, User } from 'lucide-react';
import ChatMessageContent from './ChatMessageContent';
import ChatMessageActions from './ChatMessageActions';
import MessageEditor from './MessageEditor';
import ToolCallDetails from './ToolCallDetails';
import { formatTimestamp, getMessageWarning } from './helpers';
import type { ChatMessage as ChatMessageType } from './types';

interface ChatMessageProps {
  message: ChatMessageType;
  messages: ChatMessageType[];
  isEditing: boolean;
  editContent: string;
  onStartEdit: () => void;
  onSaveEdit: () => void;
  onCancelEdit: () => void;
  onEditContentChange: (content: string) => void;
  onDelete: () => void;
}

const ChatMessage: React.FC<ChatMessageProps> = ({
  message,
  messages,
  isEditing,
  editContent,
  onStartEdit,
  onSaveEdit,
  onCancelEdit,
  onEditContentChange,
  onDelete,
}) => {
  const isUser = message.role === 'user';
  const warning = isEditing ? getMessageWarning(message.id, messages) : null;

  return (
    <div className={`flex ${isUser ? 'justify-end' : 'justify-start'} group`}>
      <div
        className={`flex gap-3 max-w-[70%] ${
          isUser ? 'flex-row-reverse' : 'flex-row'
        }`}
      >
        <div
          className={`flex-shrink-0 w-6 h-6 rounded-full flex items-center justify-center ${
            isUser ? 'bg-blue-500' : 'bg-gray-600'
          }`}
        >
          {isUser ? (
            <User className="w-3 h-3 text-white" />
          ) : (
            <Bot className="w-3 h-3 text-white" />
          )}
        </div>
        <div className="flex-1">
          <div
            className={`px-3 py-1.5 rounded-lg ${
              isUser ? 'bg-blue-500 text-white' : 'bg-white border border-gray-200'
            }`}
          >
            {isEditing ? (
              <MessageEditor
                initialContent={editContent}
                onSave={onSaveEdit}
                onCancel={onCancelEdit}
                onChange={onEditContentChange}
                showWarning={!!warning}
                warningMessage={warning || undefined}
              />
            ) : (
              <>
                <ChatMessageContent content={message.content} role={message.role} />
                {message.toolCalls && message.toolCalls.length > 0 && (
                  <ToolCallDetails toolCalls={message.toolCalls} />
                )}
                <div className="flex items-center justify-between mt-1">
                  <p
                    className={`text-[10px] ${
                      isUser ? 'text-blue-100' : 'text-gray-500'
                    }`}
                  >
                    {formatTimestamp(message.timestamp)}
                  </p>
                  {isUser && (
                    <ChatMessageActions
                      messageId={message.id}
                      onEdit={onStartEdit}
                      onDelete={onDelete}
                      isUser={isUser}
                    />
                  )}
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

export default ChatMessage;
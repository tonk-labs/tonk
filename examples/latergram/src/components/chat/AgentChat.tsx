import type React from 'react';
import { useCallback, useEffect, useRef } from 'react';
import { X } from 'lucide-react';
import { useAgentChat } from '../../lib/agent/use-agent-chat';
import { isMobile } from '../../lib/utils';
import ChatErrorBar from './ChatErrorBar';
import ChatInputBar from './ChatInputBar';
import ChatLoadingDots from './ChatLoadingDots';
import ChatMessage from './ChatMessage';

interface AgentChatProps {
  inputRef: React.RefObject<HTMLTextAreaElement>;
  onClose?: () => void;
}

const AgentChat: React.FC<AgentChatProps> = ({ inputRef, onClose }) => {
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const {
    messages,
    isLoading,
    error,
    isReady,
    sendMessage,
    clearConversation,
    stopGeneration,
    updateMessage,
    deleteMessage,
    editingMessageId,
    editContent,
    setEditingMessage,
  } = useAgentChat();

  useEffect(() => {
    const isMobileDevice = isMobile();

    if (!isMobileDevice && isReady && inputRef?.current) {
      setTimeout(() => {
        inputRef.current?.focus();
      }, 100);
    }
  }, [isReady, inputRef]);

  useEffect(() => {
    const isMobileDevice = isMobile();

    if (!isMobileDevice && !isLoading && inputRef?.current) {
      inputRef.current?.focus();
    }
  }, [isLoading, inputRef]);

  const handleStartEdit = useCallback(
    (message: (typeof messages)[0]) => {
      if (message.role === 'user') {
        setEditingMessage(message.id, message.content);
      }
    },
    [setEditingMessage]
  );

  const handleSaveEdit = useCallback(async () => {
    const id = editingMessageId;
    const content = editContent.trim();
    setEditingMessage(null);
    if (id && content) {
      await updateMessage(id, content);
    }
  }, [editingMessageId, editContent, updateMessage, setEditingMessage]);

  const handleCancelEdit = useCallback(() => {
    setEditingMessage(null);
    // Focus input after canceling edit
    setTimeout(() => {
      inputRef?.current?.focus();
    }, 50);
  }, [setEditingMessage, inputRef]);

  const handleDeleteMessage = useCallback(
    async (messageId: string) => {
      if (window.confirm('Delete this message and all following messages?')) {
        await deleteMessage(messageId);
      }
    },
    [deleteMessage]
  );

  const isMobileDevice = isMobile();

  return (
    <div className="flex flex-col h-full">
      {isMobileDevice && onClose && (
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200">
          <h2 className="text-lg font-semibold">Assistant</h2>
          <button
            type="button"
            onClick={onClose}
            className="p-2 hover:bg-gray-100 rounded-full transition-colors"
            aria-label="Close chat"
          >
            <X className="w-5 h-5" />
          </button>
        </div>
      )}
      <div className="relative flex-1 px-3 min-h-0 flex justify-center">
        {error && <ChatErrorBar error={error} />}
        <div
          ref={scrollContainerRef}
          className="flex flex-col-reverse h-full overflow-scroll gap-4 max-w-[50rem] w-full py-4 overscroll-contain"
          style={{ overscrollBehavior: 'contain' }}
        >
          <div className="flex-1 min-h-0" />

          {messages.length === 0 ? (
            <div className="text-center text-gray-500 text-xs animate-pulse">
              {isReady ? '' : 'Initializing agent service...'}
            </div>
          ) : (
            <>
              {isLoading && <ChatLoadingDots onStop={stopGeneration} />}
              {[...messages].reverse().map(message => (
                <ChatMessage
                  key={message.id}
                  message={message}
                  messages={messages}
                  isEditing={editingMessageId === message.id}
                  editContent={editContent}
                  onStartEdit={() => handleStartEdit(message)}
                  onSaveEdit={handleSaveEdit}
                  onCancelEdit={handleCancelEdit}
                  onEditContentChange={content =>
                    setEditingMessage(message.id, content)
                  }
                  onDelete={() => handleDeleteMessage(message.id)}
                />
              ))}
            </>
          )}
        </div>
      </div>

      <ChatInputBar
        onSendMessage={sendMessage}
        isLoading={isLoading}
        isReady={isReady}
        inputRef={inputRef}
        onStop={stopGeneration}
        onClear={clearConversation}
      />
    </div>
  );
};

export default AgentChat;

import React, { useCallback, useEffect } from 'react';
import { useAgentStore } from '../lib/agent/use-agent-store';
import ChatHeader from '../components/chat/ChatHeader';
import ChatErrorBar from '../components/chat/ChatErrorBar';
import ChatMessage from '../components/chat/ChatMessage';
import ChatLoadingDots from '../components/chat/ChatLoadingDots';
import ChatInputBar from '../components/chat/ChatInputBar';

interface AgentChatProps {
  inputRef: React.RefObject<HTMLTextAreaElement>;
}

const AgentChat: React.FC<AgentChatProps> = ({
  inputRef,
}) => {
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
  } = useAgentStore();

  // Auto-focus input when component mounts or when ready
  useEffect(() => {
    if (isReady && inputRef?.current) {
      // Small delay to ensure the drawer is fully open
      setTimeout(() => {
        inputRef.current?.focus();
      }, 100);
    }
  }, [isReady, inputRef]);

  // Focus input after sending a message
  useEffect(() => {
    if (!isLoading && inputRef?.current) {
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
    if (editingMessageId && editContent.trim()) {
      await updateMessage(editingMessageId, editContent);
      // Reset editing state after successful update
      setEditingMessage(null);
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

  return (
    <div className="flex flex-col h-full">
      <div className="relative flex-1 px-3 min-h-0 flex">
        {error && <ChatErrorBar error={error} />}
        <div className="flex flex-col-reverse reverse h-full overflow-scroll gap-4 max-w-full py-4">
          <div className="flex-1 min-h-0" />
          
          {messages.length === 0 ? (
            <div className="text-center text-gray-500 text-xs animate-pulse">
              {isReady
                ? ''
                : 'Initializing agent service...'}
            </div>
          ) : (
            <>{isLoading && <ChatLoadingDots onStop={stopGeneration} />}
            {[...messages]
              .reverse()
              .map(message => (
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
          </>)}

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

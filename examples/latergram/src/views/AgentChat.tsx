import React, { useCallback } from 'react';
import { useAgentStore } from '../lib/agent/use-agent-store';
import ChatHeader from '../components/chat/ChatHeader';
import ChatErrorBar from '../components/chat/ChatErrorBar';
import ChatMessage from '../components/chat/ChatMessage';
import ChatLoadingDots from '../components/chat/ChatLoadingDots';
import ChatInputBar from '../components/chat/ChatInputBar';

const AgentChat: React.FC = () => {
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

  const handleStartEdit = useCallback(
    (message: typeof messages[0]) => {
      if (message.role === 'user') {
        setEditingMessage(message.id, message.content);
      }
    },
    [setEditingMessage]
  );

  const handleSaveEdit = useCallback(async () => {
    if (editingMessageId && editContent.trim()) {
      await updateMessage(editingMessageId, editContent);
      setEditingMessage(null);
    }
  }, [editingMessageId, editContent, updateMessage, setEditingMessage]);

  const handleCancelEdit = useCallback(() => {
    setEditingMessage(null);
  }, [setEditingMessage]);

  const handleDeleteMessage = useCallback(
    async (messageId: string) => {
      if (window.confirm('Delete this message and all following messages?')) {
        await deleteMessage(messageId);
      }
    },
    [deleteMessage]
  );

  return (
    <div className="flex flex-col h-full bg-gray-50 overflow-hidden">
      <ChatHeader
        isReady={isReady}
        messageCount={messages.length}
        onClear={clearConversation}
      />

      {error && <ChatErrorBar error={error} />}

      <div className="flex-1 overflow-y-auto flex flex-col-reverse px-3 py-3 min-h-0">
        <div className="max-w-3xl mx-auto w-full flex flex-col gap-3">
          {messages.length === 0 ? (
            <div className="text-center text-gray-500 text-xs">
              {isReady
                ? 'Start a conversation by sending a message below'
                : 'Initializing agent service...'}
            </div>
          ) : (
            messages.map((message) => (
              <ChatMessage
                key={message.id}
                message={message}
                messages={messages}
                isEditing={editingMessageId === message.id}
                editContent={editContent}
                onStartEdit={() => handleStartEdit(message)}
                onSaveEdit={handleSaveEdit}
                onCancelEdit={handleCancelEdit}
                onEditContentChange={(content) => setEditingMessage(message.id, content)}
                onDelete={() => handleDeleteMessage(message.id)}
              />
            ))
          )}

          {isLoading && <ChatLoadingDots onStop={stopGeneration} />}
        </div>
      </div>

      <ChatInputBar
        onSendMessage={sendMessage}
        isLoading={isLoading}
        isReady={isReady}
      />
    </div>
  );
};

export default AgentChat;
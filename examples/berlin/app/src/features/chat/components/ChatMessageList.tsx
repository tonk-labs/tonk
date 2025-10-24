import { useEffect, useRef } from 'react';
import { useChat } from '../stores/chatStore';
import { usePresence } from '../../presence/stores/presenceStore';
import { ChatMessage } from './ChatMessage';
import { ChatTypingIndicator } from './ChatTypingIndicator';

export function ChatMessageList() {
  const { messages } = useChat();
  const { currentUserId } = usePresence();
  const scrollRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when new messages arrive
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  return (
    <div
      ref={scrollRef}
      className="flex-1 overflow-y-auto p-3 bg-background"
    >
      {messages.length === 0 ? (
        <div className="flex items-center justify-center h-full text-muted-foreground font-mono text-sm">
          No messages yet. Start the conversation!
        </div>
      ) : (
        messages.map((message) => (
          <ChatMessage
            key={message.id}
            message={message}
            isCurrentUser={message.userId === currentUserId}
          />
        ))
      )}
      <ChatTypingIndicator />
    </div>
  );
}

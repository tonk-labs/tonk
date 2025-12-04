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
  // biome-ignore lint/correctness/useExhaustiveDependencies: Only scroll on message count change, not full messages array
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages.length]);

  return (
    <div ref={scrollRef} className="flex-1 overflow-y-auto p-3 bg-white dark:bg-night-900">
      {messages.length === 0 ? (
        <div className="flex items-center justify-center h-full text-night-500 dark:text-night-400 font-mono text-sm">
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

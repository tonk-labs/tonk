import { useCallback, useEffect, useRef, useState } from 'react';
import { Button } from '../../../components/ui/button/button';
import { Input } from '../../editor/components/tiptap-ui-primitive/input/input';
import { usePresence } from '../../presence/stores/presenceStore';
import { useChat } from '../stores/chatStore';

export function ChatInput() {
  const [text, setText] = useState('');
  const { addMessage, setUserTyping } = useChat();
  const { currentUserId } = usePresence();
  const typingTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleTyping = useCallback(() => {
    if (!currentUserId) return;

    // Set typing to true
    setUserTyping(currentUserId, true);

    // Clear existing timeout
    if (typingTimeoutRef.current) {
      clearTimeout(typingTimeoutRef.current);
    }

    // Auto-clear typing after 3 seconds
    typingTimeoutRef.current = setTimeout(() => {
      setUserTyping(currentUserId, false);
    }, 3000);
  }, [currentUserId, setUserTyping]);

  const handleSend = useCallback(() => {
    if (!text.trim() || !currentUserId) return;

    const message = {
      id: crypto.randomUUID(),
      userId: currentUserId,
      text: text.trim(),
      timestamp: Date.now(),
      reactions: [],
    };

    addMessage(message);
    setText('');
    setUserTyping(currentUserId, false);

    // Clear typing timeout
    if (typingTimeoutRef.current) {
      clearTimeout(typingTimeoutRef.current);
      typingTimeoutRef.current = null;
    }

    // TODO: Broadcast message via presence system
  }, [text, currentUserId, addMessage, setUserTyping]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend]
  );

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (typingTimeoutRef.current) {
        clearTimeout(typingTimeoutRef.current);
      }
    };
  }, []);

  return (
    <div className="p-3 border-t border-night-200 dark:border-night-700 bg-white dark:bg-night-900">
      <div className="flex gap-2">
        <Input
          value={text}
          onChange={e => {
            setText(e.target.value);
            handleTyping();
          }}
          onKeyDown={handleKeyDown}
          placeholder="Type a message..."
          className="flex-1 font-mono"
        />
        <Button onClick={handleSend} disabled={!text.trim()} size="default">
          Send
        </Button>
      </div>
    </div>
  );
}

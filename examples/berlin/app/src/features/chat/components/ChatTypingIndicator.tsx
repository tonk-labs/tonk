import { Badge } from '../../editor/components/tiptap-ui-primitive/badge/badge';
import { usePresence } from '../../presence/stores/presenceStore';
import { useChat } from '../stores/chatStore';

export function ChatTypingIndicator() {
  const { typingUsers } = useChat();
  const { getActiveUsers } = usePresence();
  const users = getActiveUsers();

  if (typingUsers.size === 0) return null;

  const typingUserNames = Array.from(typingUsers)
    .map((userId) => {
      const user = users.find((u) => u.id === userId);
      return user?.name || 'Unknown';
    })
    .slice(0, 3); // Show max 3 names

  const displayText =
    typingUsers.size === 1
      ? `${typingUserNames[0]} is typing...`
      : typingUsers.size === 2
      ? `${typingUserNames[0]} and ${typingUserNames[1]} are typing...`
      : `${typingUserNames.slice(0, 2).join(', ')} and ${typingUsers.size - 2} others are typing...`;

  return (
    <div className="px-3 py-2">
      <Badge variant="secondary" className="text-xs font-mono">
        {displayText}
      </Badge>
    </div>
  );
}

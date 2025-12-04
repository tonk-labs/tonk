import { Badge } from '../../editor/components/tiptap-ui-primitive/badge/badge';
import { usePresence, type User } from '../../presence/stores/presenceStore';
import { useChat } from '../stores/chatStore';

export function ChatTypingIndicator() {
  const { typingUsers } = useChat();
  const { getOnlineUsers } = usePresence();
  const users = getOnlineUsers();

  const typingUserIds = Object.keys(typingUsers);
  if (typingUserIds.length === 0) return null;

  const typingUserNames = typingUserIds
    .map(userId => {
      const user = users.find((u: User) => u.id === userId);
      return user?.name || 'Unknown';
    })
    .slice(0, 3); // Show max 3 names

  const displayText =
    typingUserIds.length === 1
      ? `${typingUserNames[0]} is typing...`
      : typingUserIds.length === 2
        ? `${typingUserNames[0]} and ${typingUserNames[1]} are typing...`
        : `${typingUserNames.slice(0, 2).join(', ')} and ${typingUserIds.length - 2} others are typing...`;

  return (
    <div className="px-3 py-2">
      <Badge variant="gray" className="text-xs font-mono">
        {displayText}
      </Badge>
    </div>
  );
}

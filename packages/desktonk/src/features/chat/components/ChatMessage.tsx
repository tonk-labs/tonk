import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '../../editor/components/tiptap-ui-primitive/tooltip/tooltip';
import { type User, usePresence } from '../../presence/stores/presenceStore';
import { getInitials } from '../../presence/utils/userGeneration';
import type { ChatMessage as ChatMessageType } from '../types';

interface ChatMessageProps {
  message: ChatMessageType;
  isCurrentUser: boolean;
}

export function ChatMessage({ message, isCurrentUser }: ChatMessageProps) {
  const { getOnlineUsers } = usePresence();
  const users = getOnlineUsers();
  const user = users.find((u: User) => u.id === message.userId);

  const userName = user?.name || 'Unknown';
  const userColor = user?.color || '#888888';
  const userInitials = getInitials(userName);

  const formattedTime = new Date(message.timestamp).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  });

  return (
    <div
      className={`flex gap-2 mb-3 ${isCurrentUser ? 'flex-row-reverse' : 'flex-row'}`}
    >
      {/* Avatar */}
      <Tooltip placement="top" delay={300}>
        <TooltipTrigger asChild>
          <div
            className="w-8 h-8 rounded-full flex items-center justify-center text-white text-xs font-medium shrink-0"
            style={{ backgroundColor: userColor }}
          >
            {userInitials}
          </div>
        </TooltipTrigger>
        <TooltipContent>{userName}</TooltipContent>
      </Tooltip>

      {/* Message content */}
      <div
        className={`flex flex-col gap-1 max-w-[70%] ${isCurrentUser ? 'items-end' : 'items-start'}`}
      >
        <div
          className={`rounded-lg px-3 py-2 ${
            isCurrentUser
              ? 'bg-blue-500 text-white'
              : 'bg-night-100 dark:bg-night-800 border border-night-200 dark:border-night-700'
          }`}
        >
          <p className="text-sm font-mono whitespace-pre-wrap break-words">
            {message.text}
          </p>
        </div>

        {/* Timestamp */}
        <Tooltip placement="top" delay={300}>
          <TooltipTrigger asChild>
            <span className="text-xs text-night-500 dark:text-night-400 font-mono cursor-default">
              {formattedTime}
            </span>
          </TooltipTrigger>
          <TooltipContent>
            {new Date(message.timestamp).toLocaleString()}
          </TooltipContent>
        </Tooltip>

        {/* Reactions (placeholder) */}
        {message.reactions.length > 0 && (
          <div className="flex gap-1 flex-wrap">
            {message.reactions.map(reaction => (
              <span
                key={reaction.emoji}
                className="text-xs bg-accent px-2 py-1 rounded-full font-mono"
              >
                {reaction.emoji} {reaction.userIds.length}
              </span>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

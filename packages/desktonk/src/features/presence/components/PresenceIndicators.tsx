import { usePresence } from '../stores/presenceStore';
import { getInitials } from '../utils/userGeneration';
import {
  Tooltip,
  TooltipTrigger,
  TooltipContent,
} from '@/features/editor/components/tiptap-ui-primitive/tooltip/tooltip';

interface PresenceIndicatorsProps {
  className?: string;
  maxVisible?: number;
}

export const PresenceIndicators = ({ className = '', maxVisible }: PresenceIndicatorsProps) => {
  const { getOnlineUsers } = usePresence();
  const activeUsers = getOnlineUsers();

  // Apply maxVisible limit
  const visibleUsers = maxVisible ? activeUsers.slice(0, maxVisible) : activeUsers;
  const hiddenCount =
    maxVisible && activeUsers.length > maxVisible ? activeUsers.length - maxVisible : 0;

  if (activeUsers.length === 0) {
    return null;
  }

  return (
    <div className={`flex font-mono items-center ${className}`}>
      {visibleUsers.map((user, index) => (
        <Tooltip key={user.id} placement="bottom" delay={300}>
          <TooltipTrigger asChild>
            <div
              className="w-7 h-7 rounded-full flex items-center justify-center text-white text-xs font-medium border-2 bg-white cursor-default"
              style={{
                backgroundColor: user.color,
                borderColor: user.color,
                marginLeft: index > 0 ? '-8px' : '0',
              }}
            >
              {getInitials(user.name)}
            </div>
          </TooltipTrigger>
          <TooltipContent>{user.name}</TooltipContent>
        </Tooltip>
      ))}

      {hiddenCount > 0 && (
        <div className="w-7 h-7 rounded-full flex items-center justify-center bg-gray-300 text-gray-700 text-xs font-medium border-2 border-white -ml-2">
          +{hiddenCount}
        </div>
      )}
    </div>
  );
};

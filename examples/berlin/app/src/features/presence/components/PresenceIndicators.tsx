import { useEffect } from 'react';
import { usePresence, startPresenceCleanup, stopPresenceCleanup } from '../stores/presenceStore';
import { getInitials } from '../utils/userGeneration';

interface PresenceIndicatorsProps {
  className?: string;
  maxVisible?: number;
}

export const PresenceIndicators = ({
  className = '',
  maxVisible
}: PresenceIndicatorsProps) => {
  const { getActiveUsers } = usePresence();
  const activeUsers = getActiveUsers();

  // Manage cleanup timer lifecycle
  useEffect(() => {
    startPresenceCleanup();
    return () => {
      stopPresenceCleanup();
    };
  }, []);

  // Apply maxVisible limit
  const visibleUsers = maxVisible
    ? activeUsers.slice(0, maxVisible)
    : activeUsers;
  const hiddenCount = maxVisible && activeUsers.length > maxVisible
    ? activeUsers.length - maxVisible
    : 0;

  if (activeUsers.length === 0) {
    return null;
  }

  return (
    <div className={`flex items-center ${className}`}>
      {visibleUsers.map((user, index) => (
        <div
          key={user.id}
          className="relative group"
          style={{ marginLeft: index > 0 ? '-8px' : '0' }}
        >
          <div
            className="w-8 h-8 rounded-full flex items-center justify-center text-white text-sm font-medium border-2 border-white shadow-sm cursor-default"
            style={{ backgroundColor: user.color }}
            title={user.name}
          >
            {getInitials(user.name)}
          </div>

          {/* Tooltip */}
          <div className="absolute bottom-full left-1/2 transform -translate-x-1/2 mb-2 px-2 py-1 bg-gray-900 text-white text-xs rounded whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
            {user.name}
          </div>
        </div>
      ))}

      {hiddenCount > 0 && (
        <div className="w-8 h-8 rounded-full flex items-center justify-center bg-gray-300 text-gray-700 text-xs font-medium border-2 border-white shadow-sm ml-[-8px]">
          +{hiddenCount}
        </div>
      )}
    </div>
  );
};

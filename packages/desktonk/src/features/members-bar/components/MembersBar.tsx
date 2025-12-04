import { usePresence } from '@/features/presence/stores/presenceStore';
import { MemberItem } from './MemberItem';
import { useEffect } from 'react';

export function MembersBar() {
  const { getOnlineUsers, getOfflineUsers, getCurrentUser } = usePresence();
  const onlineUsers = getOnlineUsers();
  const offlineUsers = getOfflineUsers();
  const currentUser = getCurrentUser();

  useEffect(() => {
    console.log(`[users online]: ${onlineUsers.length} \n[users offline]: ${offlineUsers.length}`);
  }, [onlineUsers, offlineUsers]);

  return (
    <div className="w-[280px] shrink-0 overflow-hidden z-50">
      <div className="h-full bg-night-50 dark:bg-night-950 border-l border-night-200 dark:border-night-800 flex flex-col overflow-hidden">
        {/* Header */}
        <div className="p-4 border-b border-night-200 dark:border-night-800 flex-shrink-0">
          <div className="text-xs font-semibold text-night-600 dark:text-night-100">MEMBERS</div>
          <div className="text-[11px] text-night-400 dark:text-night-400 mt-1">
            {onlineUsers.length} online
          </div>
        </div>

        {/* Members list */}
        <div className="flex-1 overflow-y-auto overflow-x-hidden px-4 gap">
          {/* Online section */}
          {onlineUsers.length > 0 && (
            <div className="flex flex-col mt-4 gap-2">
              <div className="pb-2 text-[11px] font-semibold text-night-600 dark:text-night-100 uppercase">
                Online — {onlineUsers.length}
              </div>
              {onlineUsers
                .sort((a, b) => a.name.localeCompare(b.name))
                .map((user) => (
                  <MemberItem key={user.id} user={user} online />
                ))}
            </div>
          )}

          {/* Offline section */}
          {offlineUsers.length > 0 && (
            <div className="flex flex-col mt-4 gap-2">
              <div className="pb-2 text-[11px] font-semibold text-night-600 dark:text-night-100 uppercase">
                Offline — {offlineUsers.length}
              </div>
              {offlineUsers
                .sort((a, b) => a.name.localeCompare(b.name))
                .map((user) => (
                  <MemberItem key={user.id} user={user} />
                ))}
            </div>
          )}
        </div>
        <div className="px-4 pt-4 border-t border-night-200 dark:border-night-800 bg-night-100 dark:bg-night-900">
          {currentUser && (
            <MemberItem key={currentUser.id} user={currentUser} online={true} className="pb-4" />
          )}
        </div>
      </div>
    </div>
  );
}

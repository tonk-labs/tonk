import type { User } from '@/features/presence/stores/presenceStore';
import { getInitials } from '@/features/presence/utils/userGeneration';
import { cn } from '@/lib/utils';

interface MemberItemProps {
  user: User;
  online?: boolean;
  className?: string;
}

export function MemberItem({ user, online = false, className }: MemberItemProps) {
  return (
    <div className={cn('flex items-center gap-3', !online && 'opacity-50', className)}>
      {/* Avatar */}
      <div
        className="w-8 h-8 rounded-full flex items-center justify-center text-white text-sm font-medium flex-shrink-0"
        style={{ backgroundColor: user.color }}
      >
        {getInitials(user.name)}
      </div>

      {/* Name */}
      <span className="text-sm font-medium flex-1 truncate text-night-900 dark:text-night-100">
        {user.name}
      </span>

      {/* Status dot */}
      {online && <div className="w-2 h-2 rounded-full bg-status-online flex-shrink-0" />}
    </div>
  );
}

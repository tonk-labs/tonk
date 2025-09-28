import React, { ReactNode } from 'react';
import { Trash2 } from 'lucide-react';

interface SidebarItemProps {
  selected: boolean;
  onClick: () => void;
  onDelete?: () => void;
  icon?: ReactNode;
  title: string;
  subtitle?: string;
  status?: 'success' | 'error' | 'loading';
  error?: string;
  color?: 'blue' | 'purple' | 'green'; // Theme color for selected state
  className?: string;
  canDelete?: boolean;
  children?: ReactNode; // For custom content
}

export const SidebarItem: React.FC<SidebarItemProps> = ({
  selected,
  onClick,
  onDelete,
  icon,
  title,
  subtitle,
  status,
  error,
  color = 'blue',
  className = '',
  canDelete = true,
  children,
}) => {
  const handleDelete = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (onDelete && confirm(`Are you sure you want to delete ${title}?`)) {
      onDelete();
    }
  };

  const selectedClasses = {
    blue: 'bg-blue-50 border border-blue-200',
    purple: 'bg-purple-50 border border-purple-200',
    green: 'bg-green-50 border border-green-200',
  };

  const statusColors = {
    success: 'border-green-200 bg-green-50',
    error: 'border-red-200 bg-red-50',
    loading: 'border-blue-200 bg-blue-50',
  };

  const baseClasses = `
    p-2 mb-1 rounded cursor-pointer transition-all group
    ${selected ? selectedClasses[color] : 'hover:bg-gray-50 border border-transparent'}
    ${!selected && status ? statusColors[status] : ''}
    ${className}
  `;

  return (
    <div onClick={onClick} className={baseClasses}>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 flex-1 min-w-0">
          {icon && <div className="flex-shrink-0">{icon}</div>}
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium text-gray-800 truncate">
              {title}
            </div>
            {subtitle && (
              <div className="text-xs text-gray-500 truncate">
                {subtitle}
              </div>
            )}
            {error && status === 'error' && (
              <div className="text-xs text-red-600 truncate mt-1">
                {error}
              </div>
            )}
          </div>
        </div>
        {onDelete && canDelete && (
          <div
            onClick={handleDelete}
            className="opacity-0 group-hover:opacity-100 p-1 hover:bg-red-100 rounded transition-all"
            title="Delete"
          >
            <Trash2 className="w-3 h-3 text-red-500" />
          </div>
        )}
      </div>
      {children}
    </div>
  );
};
import React, { ReactNode } from 'react';

interface EmptyStateProps {
  icon?: ReactNode;
  title: string;
  subtitle?: string;
  actionText?: string;
  onAction?: () => void;
  className?: string;
}

export const EmptyState: React.FC<EmptyStateProps> = ({
  icon,
  title,
  subtitle,
  actionText,
  onAction,
  className = '',
}) => {
  return (
    <div className={`flex flex-col items-center justify-center py-8 px-4 text-center ${className}`}>
      {icon && (
        <div className="mb-3">
          {icon}
        </div>
      )}
      <p className="text-sm text-gray-500 font-medium">{title}</p>
      {subtitle && (
        <p className="text-xs text-gray-400 mt-1">{subtitle}</p>
      )}
      {actionText && onAction && (
        <button
          type="button"
          onClick={onAction}
          className="text-xs text-blue-500 hover:text-blue-600 mt-4"
        >
          {actionText}
        </button>
      )}
    </div>
  );
};
import React, { ReactNode } from 'react';
import { Plus } from 'lucide-react';

interface EditorSidebarProps {
  title?: string;
  onCreateClick?: () => void;
  children: ReactNode;
}

export const EditorSidebar: React.FC<EditorSidebarProps> = ({
  title,
  onCreateClick,
  children,
}) => {
  return (
    <div className="w-64 bg-white border-r border-gray-200 flex flex-col h-full">
      {title && (
        <div className="p-4 border-b border-gray-200">
          <div className="flex items-center justify-between mb-3">
            <h3 className="font-semibold text-gray-800">{title}</h3>
            {onCreateClick && (
              <button
                type="button"
                onClick={onCreateClick}
                className="p-1 hover:bg-gray-100 rounded transition-colors"
                title={`Create new ${title.toLowerCase().slice(0, -1)}`}
              >
                <Plus className="w-4 h-4 text-gray-600" />
              </button>
            )}
          </div>
        </div>
      )}
      {/* Content area for search, list items, etc. */}
      {children}
    </div>
  );
};

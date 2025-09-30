import {
  File,
  FilePlus,
  Folder,
  FolderOpen,
  FolderPlus,
  Trash2,
} from 'lucide-react';
import type React from 'react';
import type { TreeItem, TreeItemRenderContext } from 'react-complex-tree';
import type { VFSTreeItemData } from './VFSDataProvider';

interface FileTreeNodeProps {
  item: TreeItem<VFSTreeItemData>;
  context: TreeItemRenderContext;
  onDelete?: (path: string) => void;
  onCreateFile?: (parentPath: string) => void;
  onCreateDirectory?: (parentPath: string) => void;
}

export const FileTreeNode: React.FC<FileTreeNodeProps> = ({
  item,
  context,
  onDelete,
  onCreateFile,
  onCreateDirectory,
}) => {
  const { isExpanded, isSelected, isRenaming } = context;
  const { name, path, type } = item.data;

  const getIcon = () => {
    if (type === 'directory') {
      return isExpanded ? (
        <FolderOpen className="w-4 h-4 text-blue-500" />
      ) : (
        <Folder className="w-4 h-4 text-blue-500" />
      );
    }
    return <File className="w-4 h-4 text-gray-500" />;
  };

  const getFileExtension = (filename: string): string => {
    const parts = filename.split('.');
    return parts.length > 1 ? parts.pop()?.toLowerCase() || '' : '';
  };

  const getFileIcon = () => {
    if (type === 'directory') {
      return getIcon();
    }

    const extension = getFileExtension(name);
    const iconClass = 'w-4 h-4';

    // Different colors based on file type
    switch (extension) {
      case 'ts':
      case 'tsx':
        return <File className={`${iconClass} text-blue-600`} />;
      case 'js':
      case 'jsx':
        return <File className={`${iconClass} text-yellow-600`} />;
      case 'css':
      case 'scss':
      case 'sass':
        return <File className={`${iconClass} text-pink-600`} />;
      case 'html':
        return <File className={`${iconClass} text-orange-600`} />;
      case 'json':
        return <File className={`${iconClass} text-green-600`} />;
      case 'md':
        return <File className={`${iconClass} text-purple-600`} />;
      case 'py':
        return <File className={`${iconClass} text-green-700`} />;
      case 'go':
        return <File className={`${iconClass} text-cyan-600`} />;
      case 'rs':
        return <File className={`${iconClass} text-orange-700`} />;
      default:
        return <File className={`${iconClass} text-gray-500`} />;
    }
  };

  const handleDelete = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (window.confirm(`Are you sure you want to delete "${name}"?`)) {
      onDelete?.(path);
    }
  };

  const handleCreateFile = (e: React.MouseEvent) => {
    e.stopPropagation();
    onCreateFile?.(path);
  };

  const handleCreateDirectory = (e: React.MouseEvent) => {
    e.stopPropagation();
    onCreateDirectory?.(path);
  };

  return (
    <div
      className={`
        flex items-center gap-2 pl-2 py-1 text-sm cursor-pointer group relative w-full
        ${isRenaming ? 'bg-yellow-50' : ''}
      `}
      style={{
        paddingLeft: `0px`,
      }}
    >
      {/* File/Folder Icon */}
      <span className="flex-shrink-0">{getFileIcon()}</span>

      {/* File/Folder Name */}
      <span
        className={`
          flex-1 truncate
          ${isSelected ? 'font-medium' : ''}
          ${name.startsWith('.') ? 'text-gray-400' : ''}
        `}
        title={name}
      >
        {name}
      </span>

      {/* Action Buttons */}
      <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100">
        {/* Create actions for directories */}
        {type === 'directory' && (
          <>
            <button
              type="button"
              onClick={handleCreateFile}
              className="p-1 hover:bg-blue-100 rounded transition-colors"
              title="Create new file"
            >
              <FilePlus className="w-3 h-3 text-blue-600" />
            </button>
            <button
              type="button"
              onClick={handleCreateDirectory}
              className="p-1 hover:bg-blue-100 rounded transition-colors"
              title="Create new directory"
            >
              <FolderPlus className="w-3 h-3 text-blue-600" />
            </button>
          </>
        )}
        <div className="flex flex-grow" />

        {/* Delete button (not for root) */}
        {path !== '/' && (
          <button
            type="button"
            onClick={handleDelete}
            className="p-1 hover:bg-red-100 rounded transition-colors"
            title={`Delete ${name}`}
          >
            <Trash2 className="w-3 h-3 text-red-500" />
          </button>
        )}
      </div>
    </div>
  );
};

// Helper component for custom rendering with react-complex-tree
export const renderFileTreeNode = (
  onDelete?: (path: string) => void,
  onCreateFile?: (parentPath: string) => void,
  onCreateDirectory?: (parentPath: string) => void
) => {
  return ({
    item,
    context,
  }: {
    item: TreeItem<VFSTreeItemData>;
    context: TreeItemRenderContext;
  }) => (
    <FileTreeNode
      item={item}
      context={context}
      onDelete={onDelete}
      onCreateFile={onCreateFile}
      onCreateDirectory={onCreateDirectory}
    />
  );
};

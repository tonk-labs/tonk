import React from 'react';
import { DirectoryEntry, NodeMetadata } from '@tonk/core';
import { TonkService } from '../services/tonkService';

interface FileItemProps {
  item: DirectoryEntry;
  metadata: NodeMetadata;
  currentPath: string;
  onNavigate: (path: string) => void;
  onDelete: (path: string) => void;
}

const FileItem: React.FC<FileItemProps> = ({
  item,
  metadata,
  currentPath,
  onNavigate,
  onDelete,
}) => {
  const itemPath = TonkService.joinPath(currentPath, item.name);

  const handleClick = () => {
    if (item.type === 'directory') {
      onNavigate(itemPath);
    } else {
      // For files, navigate to a view that shows the file content
      onNavigate(`/view?path=${encodeURIComponent(itemPath)}`);
    }
  };

  const handleDelete = (e: React.MouseEvent) => {
    e.stopPropagation();
    onDelete(itemPath);
  };

  // Function to get file icon based on file extension
  const getFileIcon = () => {
    if (item.type === 'directory') {
      return (
        <svg
          className="w-10 h-10 text-[#1d9bf0]"
          fill="currentColor"
          viewBox="0 0 24 24"
        >
          <path d="M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z" />
        </svg>
      );
    }

    // Get file extension
    const extension = item.name.split('.').pop()?.toLowerCase() || '';

    // Return icon based on file extension
    switch (extension) {
      case 'pdf':
        return (
          <svg
            className="w-10 h-10 text-[#ff3b30]"
            fill="currentColor"
            viewBox="0 0 24 24"
          >
            <path d="M20 2H8c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-8.5 7.5c0 .83-.67 1.5-1.5 1.5H9v1.25c0 .41-.34.75-.75.75s-.75-.34-.75-.75V8c0-.55.45-1 1-1H10c.83 0 1.5.67 1.5 1.5v1zm5 2c0 .83-.67 1.5-1.5 1.5h-2c-.28 0-.5-.22-.5-.5v-5c0-.28.22-.5.5-.5h2c.83 0 1.5.67 1.5 1.5v3zm4-3.75c0 .41-.34.75-.75.75H19v1h.75c.41 0 .75.34.75.75s-.34.75-.75.75H19v1.25c0 .41-.34.75-.75.75s-.75-.34-.75-.75V8c0-.55.45-1 1-1h1.25c.41 0 .75.34.75.75zM9 9.5h1v-1H9v1zM3 6c-.55 0-1 .45-1 1v13c0 1.1.9 2 2 2h13c.55 0 1-.45 1-1s-.45-1-1-1H5c-.55 0-1-.45-1-1V7c0-.55-.45-1-1-1zm11 5.5h1v-3h-1v3z" />
          </svg>
        );
      case 'jpg':
      case 'jpeg':
      case 'png':
      case 'gif':
        return (
          <svg
            className="w-10 h-10 text-[#34c759]"
            fill="currentColor"
            viewBox="0 0 24 24"
          >
            <path d="M21 19V5c0-1.1-.9-2-2-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2zM8.5 13.5l2.5 3.01L14.5 12l4.5 6H5l3.5-4.5z" />
          </svg>
        );
      case 'doc':
      case 'docx':
        return (
          <svg
            className="w-10 h-10 text-[#0066cc]"
            fill="currentColor"
            viewBox="0 0 24 24"
          >
            <path d="M14 2H6c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6zM6 20V4h7v5h5v11H6z" />
          </svg>
        );
      case 'xls':
      case 'xlsx':
        return (
          <svg
            className="w-10 h-10 text-[#34c759]"
            fill="currentColor"
            viewBox="0 0 24 24"
          >
            <path d="M14 2H6c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6zM6 20V4h7v5h5v11H6z" />
          </svg>
        );
      case 'json':
      case 'js':
      case 'ts':
      case 'jsx':
      case 'tsx':
        return (
          <svg
            className="w-10 h-10 text-[#ff9500]"
            fill="currentColor"
            viewBox="0 0 24 24"
          >
            <path d="M14 2H6c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6zM6 20V4h7v5h5v11H6z" />
          </svg>
        );
      default:
        return (
          <svg
            className="w-10 h-10 text-[#8e8e93]"
            fill="currentColor"
            viewBox="0 0 24 24"
          >
            <path d="M14 2H6c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6zM6 20V4h7v5h5v11H6z" />
          </svg>
        );
    }
  };

  return (
    <div
      className="flex items-center justify-between p-4 hover:bg-white transition-colors cursor-pointer"
      onClick={handleClick}
    >
      <div className="flex items-center">
        <div className="mr-3">{getFileIcon()}</div>
        <div>
          <div className="font-medium text-[#1d1d1f]">{item.name}</div>
          <div className="text-xs text-[#86868b]">
            {TonkService.formatDate(metadata.modifiedAt.getTime())}
          </div>
        </div>
      </div>
      <button
        onClick={handleDelete}
        className="p-2 text-[#8e8e93] hover:text-[#ff3b30] rounded-full hover:bg-[#f5f5f7] transition-colors"
        aria-label="Delete"
      >
        <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
          <path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z" />
        </svg>
      </button>
    </div>
  );
};

export default FileItem;

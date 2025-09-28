import React, { useState, useEffect } from 'react';
import {
  Folder,
  File,
  ChevronRight,
  ChevronDown,
  Trash2,
  Eye,
  EyeOff,
} from 'lucide-react';
import { getVFSService } from '../services/vfs-service';

interface FileNode {
  name: string;
  path: string;
  type: 'file' | 'directory';
  children?: FileNode[];
  expanded?: boolean;
}

interface VFSFileBrowserProps {
  onFileSelect?: (path: string) => void;
  onFileDelete?: (path: string) => void;
  className?: string;
}

export const VFSFileBrowser: React.FC<VFSFileBrowserProps> = ({
  onFileSelect,
  onFileDelete,
  className = '',
}) => {
  const [files, setFiles] = useState<FileNode[]>([]);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(
    new Set(['/'])
  );
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showHidden, setShowHidden] = useState(false);

  const vfs = getVFSService();

  const loadDirectory = async (path: string): Promise<FileNode[]> => {
    try {
      const items = await vfs.listDirectory(path);
      console.log(`Directory listing for ${path}:`, items);

      // Deduplicate items based on name
      const seen = new Set<string>();
      const uniqueItems = items.filter((item: any) => {
        const name =
          typeof item === 'string'
            ? item
            : item?.name || item?.path?.split('/').pop();
        if (seen.has(name)) {
          return false;
        }
        seen.add(name);
        return true;
      });

      // Handle different response formats from VFS
      const processedItems = uniqueItems
        .map((item: any, index: number) => {
          // If it's a string, assume it's just the filename
          if (typeof item === 'string') {
            const fullPath = path === '/' ? `/${item}` : `${path}/${item}`;
            // Try to determine if it's a directory based on extension
            const isDirectory = !item.includes('.');
            return {
              name: item,
              path: fullPath,
              type: isDirectory ? 'directory' : 'file',
              children: isDirectory ? [] : undefined,
            };
          }
          // If it has name/path properties
          else if (item && typeof item === 'object') {
            const name = item.name || item.path?.split('/').pop() || 'unknown';
            const itemPath =
              item.path || (path === '/' ? `/${name}` : `${path}/${name}`);
            const type: 'directory' | 'file' =
              item.type === 'directory' || item.type === 'document'
                ? item.type === 'directory'
                  ? 'directory'
                  : 'file'
                : 'file';
            return {
              name,
              path: itemPath,
              type,
              children: type === 'directory' ? [] : undefined,
            } as FileNode;
          }
          return null;
        })
        .filter((item: FileNode | null): item is FileNode => {
          if (!item) return false;
          return showHidden || !item.name.startsWith('.');
        });

      return processedItems;
    } catch (err) {
      console.error(`Failed to load directory ${path}:`, err);
      return [];
    }
  };

  const loadRootFiles = async () => {
    setLoading(true);
    setError(null);
    try {
      const rootItems = await loadDirectory('/');
      setFiles(rootItems);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load files');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (vfs.isInitialized()) {
      loadRootFiles();
    }
  }, [showHidden]);

  const toggleFolder = async (node: FileNode) => {
    if (node.type !== 'directory') return;

    const newExpanded = new Set(expandedFolders);
    if (expandedFolders.has(node.path)) {
      newExpanded.delete(node.path);
      setExpandedFolders(newExpanded);
    } else {
      newExpanded.add(node.path);
      setExpandedFolders(newExpanded);

      if (node.children?.length === 0) {
        const children = await loadDirectory(node.path);
        updateNodeChildren(node.path, children);
      }
    }
  };

  const updateNodeChildren = (path: string, children: FileNode[]) => {
    setFiles(prevFiles => {
      const updateNode = (nodes: FileNode[]): FileNode[] => {
        return nodes.map(node => {
          if (node.path === path) {
            return { ...node, children };
          }
          if (node.children) {
            return { ...node, children: updateNode(node.children) };
          }
          return node;
        });
      };
      return updateNode(prevFiles);
    });
  };

  const handleFileClick = (node: FileNode) => {
    if (node.type === 'file') {
      setSelectedFile(node.path);
      onFileSelect?.(node.path);
    } else {
      toggleFolder(node);
    }
  };

  const handleDelete = async (e: React.MouseEvent, path: string) => {
    e.stopPropagation();
    if (window.confirm(`Are you sure you want to delete ${path}?`)) {
      try {
        await vfs.deleteFile(path);
        onFileDelete?.(path);
        await loadRootFiles();
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to delete file');
      }
    }
  };

  const renderNode = (
    node: FileNode,
    depth: number = 0
  ): React.ReactElement => {
    const isExpanded = expandedFolders.has(node.path);
    const isSelected = selectedFile === node.path;
    const isDirectory = node.type === 'directory';

    return (
      <div key={node.path}>
        <div
          className={`flex items-center gap-2 px-2 py-1 hover:bg-gray-100 cursor-pointer group ${
            isSelected ? 'bg-blue-50' : ''
          }`}
          style={{ paddingLeft: `${depth * 16 + 8}px` }}
          onClick={() => handleFileClick(node)}
        >
          {isDirectory ? (
            <>
              {isExpanded ? (
                <ChevronDown className="w-4 h-4 text-gray-500" />
              ) : (
                <ChevronRight className="w-4 h-4 text-gray-500" />
              )}
              <Folder className="w-4 h-4 text-blue-500" />
            </>
          ) : (
            <>
              <div className="w-4" />
              <File className="w-4 h-4 text-gray-500" />
            </>
          )}
          <span className="flex-1 text-sm text-gray-700 truncate">
            {node.name}
          </span>
          <button
            type="button"
            onClick={e => handleDelete(e, node.path)}
            className="opacity-0 group-hover:opacity-100 p-1 hover:bg-red-100 rounded transition-opacity"
            title="Delete"
          >
            <Trash2 className="w-3 h-3 text-red-500" />
          </button>
        </div>
        {isDirectory && isExpanded && node.children && (
          <div>{node.children.map(child => renderNode(child, depth + 1))}</div>
        )}
      </div>
    );
  };

  if (loading) {
    return (
      <div className={`flex items-center justify-center p-4 ${className}`}>
        <div className="text-gray-500">Loading files...</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className={`flex items-center justify-center p-4 ${className}`}>
        <div className="text-red-500">Error: {error}</div>
      </div>
    );
  }

  return (
    <div className={`flex flex-col h-full ${className}`}>
      <div className="flex items-center justify-between p-2 border-b border-gray-200">
        <h3 className="text-sm font-medium text-gray-700">Files</h3>
        <button
          type="button"
          onClick={() => setShowHidden(!showHidden)}
          className="p-1 hover:bg-gray-100 rounded transition-colors"
          title={showHidden ? 'Hide hidden files' : 'Show hidden files'}
        >
          {showHidden ? (
            <EyeOff className="w-4 h-4 text-gray-500" />
          ) : (
            <Eye className="w-4 h-4 text-gray-500" />
          )}
        </button>
      </div>
      <div className="flex-1 overflow-y-auto">
        {files.length === 0 ? (
          <div className="p-4 text-center text-gray-500 text-sm">
            No files found
          </div>
        ) : (
          files.map(node => renderNode(node))
        )}
      </div>
    </div>
  );
};

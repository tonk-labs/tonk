import React, { useState, useEffect } from 'react';
import { File, X } from 'lucide-react';
import { getVFSService } from '../services/vfs-service';

interface VFSFilePreviewProps {
  filePath: string | null;
  onClose?: () => void;
  className?: string;
}

export const VFSFilePreview: React.FC<VFSFilePreviewProps> = ({
  filePath,
  onClose,
  className = '',
}) => {
  const [content, setContent] = useState<string>('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const vfs = getVFSService();

  useEffect(() => {
    if (!filePath) {
      setContent('');
      return;
    }

    const loadFile = async () => {
      setLoading(true);
      setError(null);
      try {
        const fileContent = await vfs.readBytesAsString(filePath);
        setContent(fileContent);
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to load file');
        setContent('');
      } finally {
        setLoading(false);
      }
    };

    loadFile();
  }, [filePath]);

  if (!filePath) {
    return (
      <div className={`flex items-center justify-center h-full ${className}`}>
        <div className="text-gray-400 text-center">
          <File className="w-12 h-12 mx-auto mb-2" />
          <p>Select a file to preview</p>
        </div>
      </div>
    );
  }

  const fileName = filePath.split('/').pop() || filePath;
  const fileExtension = fileName.split('.').pop()?.toLowerCase();
  const isCode = [
    'ts',
    'tsx',
    'js',
    'jsx',
    'css',
    'html',
    'json',
    'md',
  ].includes(fileExtension || '');

  return (
    <div className={`flex flex-col h-full ${className}`}>
      <div className="flex items-center justify-between p-3 border-b border-gray-200 bg-gray-50">
        <div className="flex items-center gap-2">
          <File className="w-4 h-4 text-gray-500" />
          <span className="text-sm font-medium text-gray-700">{fileName}</span>
          <span className="text-xs text-gray-500">({filePath})</span>
        </div>
        {onClose && (
          <button
            onClick={onClose}
            className="p-1 hover:bg-gray-200 rounded transition-colors"
            title="Close preview"
          >
            <X className="w-4 h-4 text-gray-500" />
          </button>
        )}
      </div>

      <div className="flex-1 overflow-auto p-4">
        {loading ? (
          <div className="text-gray-500">Loading...</div>
        ) : error ? (
          <div className="text-red-500">Error: {error}</div>
        ) : (
          <pre
            className={`text-sm ${isCode ? 'font-mono' : 'font-sans'} whitespace-pre-wrap`}
          >
            {content}
          </pre>
        )}
      </div>
    </div>
  );
};

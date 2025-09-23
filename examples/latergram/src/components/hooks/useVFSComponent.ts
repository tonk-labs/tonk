import { useState, useEffect, useCallback } from 'react';
import { getVFSService } from '../../services/vfs-service';

export interface VFSComponentHook {
  content: string;
  isLoading: boolean;
  error: string | null;
  updateContent: (newContent: string) => Promise<void>;
  createFile: (path: string, initialContent: string) => Promise<void>;
  deleteFile: (path: string) => Promise<void>;
}

export function useVFSComponent(filePath: string | null): VFSComponentHook {
  const [content, setContent] = useState<string>('');
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  const vfs = getVFSService();

  const loadContent = useCallback(
    async (path: string) => {
      if (!vfs.isInitialized()) {
        setError('VFS not initialized');
        return;
      }

      setIsLoading(true);
      setError(null);

      try {
        const fileContent = await vfs.readFile(path);
        setContent(fileContent);
      } catch (err) {
        const errorMessage =
          err instanceof Error ? err.message : 'Failed to load file';
        setError(errorMessage);
        setContent('');
      } finally {
        setIsLoading(false);
      }
    },
    [vfs]
  );

  const updateContent = useCallback(
    async (newContent: string) => {
      if (!filePath || !vfs.isInitialized()) {
        setError('Cannot save: VFS not initialized or no file path');
        return;
      }

      try {
        await vfs.writeFile(filePath, newContent, false);
        setContent(newContent);
        setError(null);
      } catch (err) {
        const errorMessage =
          err instanceof Error ? err.message : 'Failed to save file';
        setError(errorMessage);
        throw err;
      }
    },
    [filePath, vfs]
  );

  const createFile = useCallback(
    async (path: string, initialContent: string) => {
      if (!vfs.isInitialized()) {
        setError('VFS not initialized');
        return;
      }

      try {
        await vfs.writeFile(path, initialContent, true);
        setError(null);
      } catch (err) {
        const errorMessage =
          err instanceof Error ? err.message : 'Failed to create file';
        setError(errorMessage);
        throw err;
      }
    },
    [vfs]
  );

  const deleteFile = useCallback(
    async (path: string) => {
      if (!vfs.isInitialized()) {
        setError('VFS not initialized');
        return;
      }

      try {
        await vfs.deleteFile(path);
        setError(null);
      } catch (err) {
        const errorMessage =
          err instanceof Error ? err.message : 'Failed to delete file';
        setError(errorMessage);
        throw err;
      }
    },
    [vfs]
  );

  useEffect(() => {
    if (filePath) {
      loadContent(filePath);
    } else {
      setContent('');
      setError(null);
      setIsLoading(false);
    }
  }, [filePath, loadContent]);

  return {
    content,
    isLoading,
    error,
    updateContent,
    createFile,
    deleteFile,
  };
}

import { useState, useEffect, useCallback } from 'react';
import { getVFSService } from '../../services/vfs-service';

interface UseVFSStoreReturn {
  content: string;
  isLoading: boolean;
  error: string | null;
  updateContent: (newContent: string) => Promise<void>;
  createFile: (content: string) => Promise<void>;
  deleteFile: () => Promise<void>;
}

export const useVFSStore = (filePath: string | null): UseVFSStoreReturn => {
  const [content, setContent] = useState<string>('');
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  const vfs = getVFSService();

  const loadContent = useCallback(async () => {
    if (!filePath || !vfs.isInitialized()) {
      return;
    }

    setIsLoading(true);
    setError(null);

    try {
      const fileContent = await vfs.readFile(filePath);
      setContent(fileContent || '');
    } catch (err) {
      const errorMessage =
        err instanceof Error ? err.message : 'Failed to load store';
      setError(errorMessage);
      setContent('');
    } finally {
      setIsLoading(false);
    }
  }, [filePath, vfs]);

  const updateContent = useCallback(
    async (newContent: string) => {
      if (!filePath || !vfs.isInitialized()) {
        throw new Error('VFS not ready or no file path');
      }

      try {
        await vfs.writeFile(filePath, newContent, false);
        setContent(newContent);
        setError(null);
      } catch (err) {
        const errorMessage =
          err instanceof Error ? err.message : 'Failed to update store';
        setError(errorMessage);
        throw err;
      }
    },
    [filePath, vfs]
  );

  const createFile = useCallback(
    async (initialContent: string) => {
      if (!filePath || !vfs.isInitialized()) {
        throw new Error('VFS not ready or no file path');
      }

      try {
        await vfs.writeFile(filePath, initialContent, true);
        setContent(initialContent);
        setError(null);
      } catch (err) {
        const errorMessage =
          err instanceof Error ? err.message : 'Failed to create store';
        setError(errorMessage);
        throw err;
      }
    },
    [filePath, vfs]
  );

  const deleteFile = useCallback(async () => {
    if (!filePath || !vfs.isInitialized()) {
      throw new Error('VFS not ready or no file path');
    }

    try {
      await vfs.deleteFile(filePath);
      setContent('');
      setError(null);
    } catch (err) {
      const errorMessage =
        err instanceof Error ? err.message : 'Failed to delete store';
      setError(errorMessage);
      throw err;
    }
  }, [filePath, vfs]);

  // Load content when filePath changes or VFS becomes ready
  useEffect(() => {
    if (filePath) {
      loadContent();
    } else {
      setContent('');
      setError(null);
      setIsLoading(false);
    }
  }, [loadContent, filePath]);

  // Set up file watcher for external changes
  useEffect(() => {
    if (!filePath || !vfs.isInitialized()) {
      return;
    }

    let watchId: string | null = null;

    const setupWatcher = async () => {
      try {
        watchId = await vfs.watchFile(filePath, (newContent: string) => {
          setContent(newContent);
        });
      } catch (err) {
        console.warn('Failed to set up file watcher for store:', err);
      }
    };

    setupWatcher();

    return () => {
      if (watchId) {
        vfs.unwatchFile(watchId).catch(console.warn);
      }
    };
  }, [filePath, vfs]);

  return {
    content,
    isLoading,
    error,
    updateContent,
    createFile,
    deleteFile,
  };
};

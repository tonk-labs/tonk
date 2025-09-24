import { useState, useRef, useCallback } from 'react';
import { getVFSService } from '../../services/vfs-service';

interface DiscoveryConfig {
  directory: string;
  fileExtension: string;
  extractName: (content: string, fileName: string) => string;
  onFileFound: (
    filePath: string,
    name: string,
    content: string
  ) => Promise<void>;
  checkExisting?: (filePath: string) => boolean;
}

export const useVFSDiscovery = (config: DiscoveryConfig) => {
  const [isLoaded, setIsLoaded] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isLoadingRef = useRef(false);

  const discoverFiles = useCallback(async () => {
    const vfs = getVFSService();

    if (!vfs.isInitialized() || isLoadingRef.current || isLoaded) {
      return;
    }

    isLoadingRef.current = true;
    setIsLoading(true);
    setError(null);

    try {
      const files = await vfs.listDirectory(config.directory);

      for (const fileInfo of files as any[]) {
        const fileName =
          typeof fileInfo === 'string'
            ? fileInfo
            : fileInfo.name || fileInfo.path;

        if (!fileName || !fileName.endsWith(config.fileExtension)) {
          continue;
        }

        const filePath = `${config.directory}/${fileName}`;

        // Check if already exists if checker provided
        if (config.checkExisting && config.checkExisting(filePath)) {
          continue;
        }

        try {
          const content = await vfs.readFile(filePath);
          const name = config.extractName(content, fileName);
          await config.onFileFound(filePath, name, content);
        } catch (fileError) {
          console.warn(`Failed to process file ${filePath}:`, fileError);
        }
      }
    } catch (error) {
      const errorMsg = `Failed to discover files in ${config.directory}`;
      console.warn(errorMsg, error);
      setError(errorMsg);
    } finally {
      isLoadingRef.current = false;
      setIsLoading(false);
      setIsLoaded(true);
    }
  }, [config, isLoaded]);

  return {
    discoverFiles,
    isLoading,
    isLoaded,
    error,
  };
};

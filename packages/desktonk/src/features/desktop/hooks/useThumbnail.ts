import { useEffect, useState } from 'react';
import { getVFSService } from '@/vfs-client';

/**
 * In-memory cache for loaded thumbnails.
 * Maps VFS path to base64 data URL.
 */
const thumbnailCache = new Map<string, string>();

/**
 * Set of paths currently being loaded.
 * Prevents duplicate concurrent loads for the same thumbnail.
 */
const loadingPaths = new Set<string>();

/**
 * Listeners waiting for a specific path to load.
 * Used to notify multiple components waiting for the same thumbnail.
 */
const loadListeners = new Map<string, Set<(thumbnail: string | null) => void>>();

/**
 * Listeners for cache invalidation.
 * Components subscribe to be notified when their thumbnail is invalidated.
 */
const invalidationListeners = new Map<string, Set<() => void>>();

/**
 * Hook for lazy-loading thumbnails from VFS.
 *
 * @param thumbnailPath - VFS path to the thumbnail file (e.g., /var/lib/desktonk/thumbnails/myfile.png)
 * @returns Object with thumbnail data URL and loading state
 */
export function useThumbnail(thumbnailPath: string | undefined): {
  thumbnail: string | null;
  isLoading: boolean;
} {
  const [thumbnail, setThumbnail] = useState<string | null>(() => {
    if (thumbnailPath && thumbnailCache.has(thumbnailPath)) {
      return thumbnailCache.get(thumbnailPath)!;
    }
    return null;
  });

  const [isLoading, setIsLoading] = useState(() => {
    // Not loading if we have cached thumbnail
    if (thumbnailPath && thumbnailCache.has(thumbnailPath)) return false;
    // Loading if we have a path but no cached data
    return !!thumbnailPath;
  });

  // Counter to force reload when thumbnail is invalidated
  const [reloadTrigger, setReloadTrigger] = useState(0);

  // Subscribe to invalidation events for this thumbnail path
  useEffect(() => {
    if (!thumbnailPath) return;

    const handleInvalidation = () => {
      // Clear local state and trigger reload
      setThumbnail(null);
      setIsLoading(true);
      setReloadTrigger((prev) => prev + 1);
    };

    if (!invalidationListeners.has(thumbnailPath)) {
      invalidationListeners.set(thumbnailPath, new Set());
    }
    invalidationListeners.get(thumbnailPath)!.add(handleInvalidation);

    return () => {
      invalidationListeners.get(thumbnailPath)?.delete(handleInvalidation);
      if (invalidationListeners.get(thumbnailPath)?.size === 0) {
        invalidationListeners.delete(thumbnailPath);
      }
    };
  }, [thumbnailPath]);

  useEffect(() => {
    // If no thumbnail path, nothing to load
    if (!thumbnailPath) {
      setThumbnail(null);
      setIsLoading(false);
      return;
    }

    // Check cache first (skip if reloadTrigger changed, meaning cache was invalidated)
    if (thumbnailCache.has(thumbnailPath)) {
      setThumbnail(thumbnailCache.get(thumbnailPath)!);
      setIsLoading(false);
      return;
    }

    // If already loading this path, wait for it
    if (loadingPaths.has(thumbnailPath)) {
      setIsLoading(true);

      // Register listener for when load completes
      if (!loadListeners.has(thumbnailPath)) {
        loadListeners.set(thumbnailPath, new Set());
      }
      const listeners = loadListeners.get(thumbnailPath)!;

      const listener = (loadedThumbnail: string | null) => {
        setThumbnail(loadedThumbnail);
        setIsLoading(false);
      };

      listeners.add(listener);

      return () => {
        listeners.delete(listener);
        if (listeners.size === 0) {
          loadListeners.delete(thumbnailPath);
        }
      };
    }

    // Start loading
    setIsLoading(true);
    loadingPaths.add(thumbnailPath);

    const loadThumbnail = async () => {
      try {
        const vfs = getVFSService();
        const doc = await vfs.readFile(thumbnailPath);

        // Extract thumbnail data from document
        // Thumbnail is stored as { content: { data: base64, mimeType: 'image/png' } }
        const content = doc.content as {
          data?: string;
          mimeType?: string;
        } | null;

        if (content?.data && content?.mimeType) {
          const dataUrl = `data:${content.mimeType};base64,${content.data}`;
          thumbnailCache.set(thumbnailPath, dataUrl);
          setThumbnail(dataUrl);

          // Notify other listeners
          const listeners = loadListeners.get(thumbnailPath);
          if (listeners) {
            for (const listener of listeners) {
              listener(dataUrl);
            }
          }
        } else {
          // No valid thumbnail data
          setThumbnail(null);

          // Notify other listeners
          const listeners = loadListeners.get(thumbnailPath);
          if (listeners) {
            for (const listener of listeners) {
              listener(null);
            }
          }
        }
      } catch (error) {
        console.warn(`[useThumbnail] Failed to load thumbnail from ${thumbnailPath}:`, error);
        setThumbnail(null);

        // Notify other listeners of failure
        const listeners = loadListeners.get(thumbnailPath);
        if (listeners) {
          for (const listener of listeners) {
            listener(null);
          }
        }
      } finally {
        loadingPaths.delete(thumbnailPath);
        loadListeners.delete(thumbnailPath);
        setIsLoading(false);
      }
    };

    loadThumbnail();
  }, [thumbnailPath, reloadTrigger]);

  return { thumbnail, isLoading };
}

/**
 * Clears a specific thumbnail from the cache.
 * Call this when a thumbnail is regenerated to force reload.
 * Notifies any components currently displaying this thumbnail to reload.
 */
export function invalidateThumbnailCache(thumbnailPath: string): void {
  thumbnailCache.delete(thumbnailPath);

  // Notify any components currently displaying this thumbnail
  const listeners = invalidationListeners.get(thumbnailPath);
  if (listeners) {
    for (const listener of listeners) {
      listener();
    }
  }
}

/**
 * Clears all thumbnails from the cache.
 */
export function clearThumbnailCache(): void {
  thumbnailCache.clear();
}

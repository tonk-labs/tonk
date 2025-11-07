import { useEffect, useState, useRef } from 'react';
import { useEditor, type TLShapeId } from 'tldraw';
import type { RefNode } from '@tonk/core';
import { getVFSService } from '../../../lib/vfs-service';
import { extractDesktopFile, getNextAutoLayoutPosition } from '../utils/fileMetadata';
import type { DesktopFile } from '../types';
import { syncCoordinator } from './syncCoordinator';
import { showWarning } from '../../../lib/notifications';
import { desktopSync } from '../lib/desktopSync';
import { deletionSyncControl } from './deletionSyncControl';

interface UseDesktopSyncOptions {
  canvasPersistenceReady: boolean;
}

/**
 * Directory path where desktop files are stored in VFS.
 */
export const DESKTOP_DIRECTORY = '/desktonk';

/**
 * Debounce delay for directory watch callbacks (in milliseconds).
 * Balances between responsiveness and performance during bulk file operations.
 * Too short: reload thrashing when many files change rapidly.
 * Too long: delayed UI updates, poor user experience.
 */
const DIRECTORY_WATCH_DEBOUNCE_MS = 300;

export function useDesktopSync(options: UseDesktopSyncOptions) {
  const editor = useEditor();
  const [files, setFiles] = useState<DesktopFile[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const loadInProgressRef = useRef(false);
  const isUnmountedRef = useRef(false);
  const debounceTimerRef = useRef<number | undefined>(undefined);
  const { canvasPersistenceReady } = options;

  // Subscribe to cross-tab sync messages immediately (separate from canvas persistence)
  useEffect(() => {
    const unsubscribeSync = desktopSync.subscribe((message) => {
      console.log('[useDesktopSync] Received sync message:', message);
      if (message.type === 'refresh' || message.type === 'files-changed' || message.type === 'file-added') {
        console.log('[useDesktopSync] Triggering loadDesktopFiles due to sync message');
        // Don't reload if canvas persistence isn't ready yet
        if (canvasPersistenceReady) {
          // We'll need to trigger a reload - use a ref to call the function
          window.dispatchEvent(new CustomEvent('desktop-files-changed'));
        }
      }
    });

    return () => {
      unsubscribeSync();
    };
  }, [canvasPersistenceReady]);

  useEffect(() => {
    // Wait for canvas persistence to be ready before loading desktop files
    if (!canvasPersistenceReady) {
      console.log('[useDesktopSync] Waiting for canvas persistence to be ready...');
      return;
    }
    console.log('[useDesktopSync] Canvas persistence is ready, proceeding with desktop sync');
    const vfs = getVFSService();
    let watchId: string | null = null;
    isUnmountedRef.current = false;

    async function loadDesktopFiles(): Promise<void> {
      // Check if component unmounted
      if (isUnmountedRef.current) {
        console.log('[useDesktopSync] Component unmounted, aborting load');
        return;
      }

      // Mutex: Prevent concurrent loads that could cause race conditions
      if (loadInProgressRef.current) {
        console.log('[useDesktopSync] Load already in progress, skipping concurrent call');
        return;
      }

      loadInProgressRef.current = true;
      try {
        // 1. Cancel all pending position saves to prevent race condition (CRITICAL #4)
        // This MUST happen before clearing shapes to prevent stale shape references
        // from writing incorrect positions to VFS after shapes are recreated.
        syncCoordinator.cancelAllPendingSaves();

        // 2. Pause deletion sync to prevent VFS files from being deleted during shape rebuild
        deletionSyncControl.pause();

        // 3. Clear all existing file-icon shapes before creating new ones
        // This clear-and-rebuild approach prevents:
        // - Shape accumulation (CRITICAL #2): Multiple shapes for the same file
        // - Orphaned shapes (CRITICAL #5): Shapes for deleted files remain on canvas
        // When a file is deleted, its shape is removed during this clear phase
        // and not recreated since it won't be in the VFS listing.
        const existingFileIcons = Array.from(editor.getCurrentPageShapeIds())
          .map(id => editor.getShape(id))
          .filter((shape): shape is NonNullable<typeof shape> => shape?.type === 'file-icon');

        if (existingFileIcons.length > 0) {
          editor.deleteShapes(existingFileIcons.map(s => s.id));
        }

        // 4. Load files from VFS
        // Use Promise.allSettled to handle individual file failures gracefully
        // If one file is corrupted, we still show the rest instead of failing entirely
        const entries = await vfs.listDirectory(DESKTOP_DIRECTORY);

        // Abort check after async operation
        if (isUnmountedRef.current) {
          console.log('[useDesktopSync] Component unmounted after listDirectory, aborting');
          return;
        }

        // Validate entries is an array before using
        if (!Array.isArray(entries)) {
          console.error('[useDesktopSync] Invalid directory listing:', entries);
          throw new Error('Failed to list directory: invalid response');
        }

        const refNodes = entries as RefNode[];
        const filePromises = refNodes
          .filter(entry => entry.type === 'document')
          .map(async (entry) => {
            try {
              const doc = await vfs.readFile(`${DESKTOP_DIRECTORY}/${entry.name}`);
              return extractDesktopFile(`${DESKTOP_DIRECTORY}/${entry.name}`, doc);
            } catch (error) {
              console.error(`[useDesktopSync] Failed to load file ${entry.name}:`, error);
              throw error; // Re-throw so Promise.allSettled marks it as rejected
            }
          });

        const results = await Promise.allSettled(filePromises);

        // Abort check after async operation
        if (isUnmountedRef.current) {
          console.log('[useDesktopSync] Component unmounted after file loading, aborting');
          return;
        }

        // Extract successful results and log failures
        const loadedFiles = results
          .filter((result): result is PromiseFulfilledResult<DesktopFile> => result.status === 'fulfilled')
          .map(result => result.value);

        console.log('[useDesktopSync] Loaded files:', loadedFiles.length, loadedFiles.map(f => f.name));

        const failedCount = results.filter(r => r.status === 'rejected').length;
        if (failedCount > 0) {
          console.warn(`[useDesktopSync] Failed to load ${failedCount} file(s) from desktop. See errors above.`);
          showWarning(
            `Failed to load ${failedCount} file${failedCount > 1 ? 's' : ''} from desktop. Check console for details.`,
            6000
          );
        }

        setFiles(loadedFiles);

        // 5. Create fresh shapes for each file
        // Final abort check before creating shapes (prevents crashes on unmounted editor)
        if (isUnmountedRef.current) {
          console.log('[useDesktopSync] Component unmounted before shape creation, aborting');
          return;
        }

        loadedFiles.forEach((file, index) => {
          const position = file.desktopMeta?.x && file.desktopMeta?.y
            ? { x: file.desktopMeta.x, y: file.desktopMeta.y }
            : getNextAutoLayoutPosition(index);

          // Use deterministic ID based on file path to prevent duplicate shapes
          // This ensures that the same file always gets the same shape ID
          const shapeId = `shape:file-icon:${file.path}` as const;

          console.log('[useDesktopSync] Creating shape for file:', file.name, {
            position,
            thumbnail: file.desktopMeta?.thumbnail ? `${file.desktopMeta.thumbnail.substring(0, 50)}...` : undefined,
            mimeType: file.mimeType
          });

          try {
            editor.createShape({
              id: shapeId as unknown as TLShapeId, // Type assertion needed for TLDraw's ID type
              type: 'file-icon',
              x: position.x,
              y: position.y,
              props: {
                filePath: file.path,
                fileName: file.name,
                mimeType: file.mimeType,
                customIcon: file.desktopMeta?.icon,
                thumbnail: file.desktopMeta?.thumbnail,
                appHandler: file.desktopMeta?.appHandler,
                w: 80,
                h: 100,
              },
            });
            console.log('[useDesktopSync] Successfully created shape for:', file.name);
            console.log('[useDesktopSync] Total shapes on canvas:', editor.getCurrentPageShapes().length);
          } catch (error) {
            console.error('[useDesktopSync] Failed to create shape for:', file.name, error);
          }
        });

        // 6. Resume deletion sync after all shapes are created
        deletionSyncControl.resume();

        setIsLoading(false);
      } catch (error) {
        console.error('[useDesktopSync] Failed to load desktop files:', error);
        setIsLoading(false);
      } finally {
        loadInProgressRef.current = false;
      }
    }

    // Setup directory watcher
    async function setupWatcher(): Promise<void> {
      let tempWatchId: string | null = null;
      try {
        tempWatchId = await vfs.watchDirectory(DESKTOP_DIRECTORY, (changeData) => {
          console.log('[useDesktopSync] Directory changed:', changeData);

          // Debounce rapid directory changes (e.g., build systems writing many files)
          // This prevents reload thrashing and improves performance
          clearTimeout(debounceTimerRef.current);

          debounceTimerRef.current = setTimeout(() => {
            // Check if VFS is still connected before reloading
            if (!vfs.isInitialized()) {
              console.warn('[useDesktopSync] VFS disconnected, skipping reload. Will retry when reconnected.');
              return;
            }

            // If position saves are in progress, queue reload instead of executing immediately
            // This prevents infinite loops while ensuring reloads eventually happen
            if (syncCoordinator.shouldDeferReload()) {
              console.log('Deferring reload - position save in progress, will execute after save completes');
              syncCoordinator.queueReload(() => {
                loadDesktopFiles();
              });
              return;
            }

            // No saves in progress, reload immediately
            loadDesktopFiles();
          }, DIRECTORY_WATCH_DEBOUNCE_MS);
        });
        // Only set watchId if successful
        watchId = tempWatchId;
      } catch (error) {
        console.error('[useDesktopSync] Failed to setup directory watcher:', error);
        // Clean up partial watcher if it was created before error
        if (tempWatchId) {
          vfs.unwatchDirectory(tempWatchId).catch(console.error);
        }
      }
    }

    // Subscribe to VFS connection state changes
    // This ensures we load when VFS connects, even if it wasn't ready initially
    const unsubscribeVFS = vfs.onConnectionStateChange((state) => {
      if (state === 'connected' && !isUnmountedRef.current) {
        console.log('[useDesktopSync] VFS connected, loading desktop files');
        loadDesktopFiles();
        setupWatcher();
      } else if (state === 'disconnected') {
        console.warn('[useDesktopSync] VFS disconnected');
        setIsLoading(true);
      }
    });

    // Initial load if already connected
    if (vfs.isInitialized()) {
      loadDesktopFiles();
      setupWatcher();
    }

    // Listen for custom event from broadcast subscription
    const handleFilesChanged = () => {
      console.log('[useDesktopSync] Handling desktop-files-changed event');
      loadDesktopFiles();
    };
    window.addEventListener('desktop-files-changed', handleFilesChanged);

    return () => {
      // Remove custom event listener
      window.removeEventListener('desktop-files-changed', handleFilesChanged);
      // Unsubscribe from VFS connection changes
      unsubscribeVFS();
      // Mark component as unmounted to abort any in-flight operations
      isUnmountedRef.current = true;

      // Clear debounce timer to prevent reload after unmount
      clearTimeout(debounceTimerRef.current);

      if (watchId) {
        vfs.unwatchDirectory(watchId).catch(console.error);
      }
      // Reset syncCoordinator to prevent stale state leaking between mounts
      syncCoordinator.reset();
      // Reset load mutex
      loadInProgressRef.current = false;
    };
  }, [editor, canvasPersistenceReady]);

  return { files, isLoading };
}

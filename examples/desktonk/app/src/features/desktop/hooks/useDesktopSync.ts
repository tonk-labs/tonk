import { useEffect, useState, useRef } from 'react';
import { useEditor } from 'tldraw';
import type { RefNode } from '@tonk/core';
import { getVFSService } from '../../../lib/vfs-service';
import { extractDesktopFile, getNextAutoLayoutPosition } from '../utils/fileMetadata';
import type { DesktopFile } from '../types';
import { syncCoordinator } from './syncCoordinator';

/**
 * Debounce delay for directory watch callbacks (in milliseconds).
 * Balances between responsiveness and performance during bulk file operations.
 * Too short: reload thrashing when many files change rapidly.
 * Too long: delayed UI updates, poor user experience.
 */
const DIRECTORY_WATCH_DEBOUNCE_MS = 300;

export function useDesktopSync() {
  const editor = useEditor();
  const [files, setFiles] = useState<DesktopFile[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const loadInProgressRef = useRef(false);
  const isUnmountedRef = useRef(false);
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout>>();

  useEffect(() => {
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

        // 2. Clear all existing file-icon shapes before creating new ones
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

        // 3. Load files from VFS
        // Use Promise.allSettled to handle individual file failures gracefully
        // If one file is corrupted, we still show the rest instead of failing entirely
        const entries = await vfs.listDirectory('/desktonk');

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
              const doc = await vfs.readFile(`/desktonk/${entry.name}`);
              return extractDesktopFile(`/desktonk/${entry.name}`, doc);
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
        const desktopFiles = results
          .filter((result): result is PromiseFulfilledResult<DesktopFile> => result.status === 'fulfilled')
          .map(result => result.value);

        const failedCount = results.filter(r => r.status === 'rejected').length;
        if (failedCount > 0) {
          console.warn(`[useDesktopSync] Failed to load ${failedCount} file(s) from desktop. See errors above.`);
        }

        setFiles(desktopFiles);

        // 4. Create fresh shapes for each file
        // Final abort check before creating shapes (prevents crashes on unmounted editor)
        if (isUnmountedRef.current) {
          console.log('[useDesktopSync] Component unmounted before shape creation, aborting');
          return;
        }

        desktopFiles.forEach((file, index) => {
          const position = file.desktopMeta?.x && file.desktopMeta?.y
            ? { x: file.desktopMeta.x, y: file.desktopMeta.y }
            : getNextAutoLayoutPosition(index);

          // Use deterministic ID based on file path to prevent duplicate shapes
          // This ensures that the same file always gets the same shape ID
          const shapeId = `file-icon:${file.path}`;

          editor.createShape({
            id: shapeId as any, // Type assertion needed for TLDraw's ID type
            type: 'file-icon',
            x: position.x,
            y: position.y,
            props: {
              filePath: file.path,
              fileName: file.name,
              mimeType: file.mimeType,
              customIcon: file.desktopMeta?.icon,
              appHandler: file.desktopMeta?.appHandler,
              w: 80,
              h: 100,
            },
          });
        });

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
        tempWatchId = await vfs.watchDirectory('/desktonk', (changeData) => {
          console.log('Directory changed:', changeData);

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

    return () => {
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
  }, [editor]);

  return { files, isLoading };
}

import { useEffect, useState } from 'react';
import { useEditor } from 'tldraw';
import type { RefNode } from '@tonk/core';
import { getVFSService } from '../../../lib/vfs-service';
import { extractDesktopFile, getNextAutoLayoutPosition } from '../utils/fileMetadata';
import type { DesktopFile } from '../types';
import { syncCoordinator } from './syncCoordinator';

export function useDesktopSync() {
  const editor = useEditor();
  const [files, setFiles] = useState<DesktopFile[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    const vfs = getVFSService();
    let watchId: string | null = null;

    async function loadDesktopFiles() {
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
        const entries = (await vfs.listDirectory('/desktonk')) as RefNode[];
        const filePromises = entries
          .filter(entry => entry.type === 'document')
          .map(async (entry) => {
            const doc = await vfs.readFile(`/desktonk/${entry.name}`);
            return extractDesktopFile(`/desktonk/${entry.name}`, doc);
          });

        const desktopFiles = await Promise.all(filePromises);
        setFiles(desktopFiles);

        // 4. Create fresh shapes for each file
        desktopFiles.forEach((file, index) => {
          const position = file.desktopMeta?.x && file.desktopMeta?.y
            ? { x: file.desktopMeta.x, y: file.desktopMeta.y }
            : getNextAutoLayoutPosition(index);

          editor.createShape({
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
        console.error('Failed to load desktop files:', error);
        setIsLoading(false);
      }
    }

    // Setup directory watcher
    async function setupWatcher() {
      try {
        watchId = await vfs.watchDirectory('/desktonk', (changeData) => {
          console.log('Directory changed:', changeData);

          // Skip reload if position saves are in progress to prevent infinite loop
          if (syncCoordinator.shouldSkipReload()) {
            console.log('Skipping reload - position save in progress');
            return;
          }

          // Reload files on change
          loadDesktopFiles();
        });
      } catch (error) {
        console.error('Failed to setup directory watcher:', error);
      }
    }

    if (vfs.isInitialized()) {
      loadDesktopFiles();
      setupWatcher();
    }

    return () => {
      if (watchId) {
        vfs.unwatchDirectory(watchId).catch(console.error);
      }
    };
  }, [editor]);

  return { files, isLoading };
}

import { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import { getVFSService } from '@/vfs-client';
import { getDesktopService } from '@/features/desktop/services/DesktopService';
import {
  DESKTOP_DIRECTORY,
  THUMBNAILS_DIRECTORY,
} from '@/features/desktop/constants';
import { generateTextThumbnailFromContent } from '@/features/desktop/utils/thumbnailGenerator';

export function useDockActions() {
  const navigate = useNavigate();

  const createNewNote = useCallback(async () => {
    const vfs = getVFSService();
    const desktopService = getDesktopService();

    // Generate unique filename
    const baseName = 'untitled';
    const extension = '.md';
    let counter = 1;
    let fileName = `${baseName}${extension}`;
    let filePath = `${DESKTOP_DIRECTORY}/${fileName}`;

    // Find next available filename
    while (await vfs.exists(filePath)) {
      fileName = `${baseName}-${counter}${extension}`;
      filePath = `${DESKTOP_DIRECTORY}/${fileName}`;
      counter++;
    }

    // Create empty markdown file with plain text format
    try {
      // Generate thumbnail for empty file
      const fileId = fileName.replace(/\.[^.]+$/, '');
      let thumbnailPath: string | undefined;

      const thumbnailDataUrl = await generateTextThumbnailFromContent(
        '',
        fileName
      );
      if (thumbnailDataUrl) {
        thumbnailPath = `${THUMBNAILS_DIRECTORY}/${fileId}.png`;
        const base64Data = thumbnailDataUrl.split(',')[1];

        try {
          await vfs.writeFile(
            thumbnailPath,
            {
              content: {
                data: base64Data,
                mimeType: 'image/png',
              },
            },
            true
          );
        } catch (thumbnailError) {
          console.warn(
            '[useDockActions] Failed to write thumbnail:',
            thumbnailError
          );
          thumbnailPath = undefined;
        }
      }

      // Create file with desktopMeta including thumbnailPath
      const desktopMeta = {
        mimeType: 'text/markdown',
        ...(thumbnailPath && { thumbnailPath }),
      };

      await vfs.writeFile(
        filePath,
        { content: { text: '', desktopMeta } },
        true
      );

      // Notify desktop service about new file
      await desktopService.onFileAdded(filePath);

      // Navigate to text editor with the new file
      const encodedPath = encodeURIComponent(filePath);
      navigate(`/text-editor?file=${encodedPath}`);
    } catch (error) {
      console.error('[useDockActions] Failed to create new note:', error);
    }
  }, [navigate]);

  return {
    createNewNote,
  };
}

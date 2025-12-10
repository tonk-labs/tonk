import { useCallback, useState } from 'react';
import { useEditor, useToasts } from 'tldraw';
import { getVFSService } from '@/vfs-client';
import { DESKTOP_DIRECTORY, THUMBNAILS_DIRECTORY } from '../constants';
import { getDesktopService } from '../services/DesktopService';
import { getMimeType } from '../utils/mimeResolver';
import { generateImageThumbnail, generateTextThumbnail } from '../utils/thumbnailGenerator';

/**
 * Maximum file size allowed for upload (10MB).
 * Prevents browser memory issues with very large files.
 */
const MAX_FILE_SIZE_BYTES = 10 * 1024 * 1024;

/**
 * Common extensionless text files that should be treated as text.
 */
const EXTENSIONLESS_TEXT_FILES = [
  'LICENSE',
  'README',
  'CHANGELOG',
  'CONTRIBUTING',
  'AUTHORS',
  'COPYING',
  'INSTALL',
  'NOTICE',
  'TODO',
  'MAKEFILE',
  'Makefile',
  'Dockerfile',
  'Jenkinsfile',
  'Vagrantfile',
  'Gemfile',
  'Rakefile',
];

/**
 * Hook that handles drag and drop of files onto the desktop canvas.
 * Uploads files to VFS and creates file-icon shapes at drop coordinates.
 */
export function useFileDrop() {
  const editor = useEditor();
  const { addToast, removeToast } = useToasts();
  const [isDraggingOver, setIsDraggingOver] = useState(false);

  /**
   * Reads a file's content as text or base64 depending on file type.
   */
  const readFileContent = useCallback(
    async (
      file: File
    ): Promise<string | { type: 'binary'; encoding: 'base64'; data: string; mimeType: string }> => {
      const mimeType = file.type || getMimeType(file.name);
      const fileName = file.name.toUpperCase();

      // Check if it's a known extensionless text file
      const isExtensionlessText = EXTENSIONLESS_TEXT_FILES.some(
        (name) => fileName === name || fileName === name.toLowerCase()
      );

      // For text files, read as text
      if (
        mimeType.startsWith('text/') ||
        mimeType === 'application/json' ||
        mimeType === 'application/x-yaml' ||
        mimeType === 'application/yaml' ||
        mimeType === 'text/yaml' ||
        mimeType === 'text/x-yaml' ||
        isExtensionlessText
      ) {
        return new Promise((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => resolve(reader.result as string);
          reader.onerror = () => reject(new Error('Failed to read file'));
          reader.readAsText(file);
        });
      }

      // For binary files, read as base64 and return as object
      return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => {
          const base64 = (reader.result as string).split(',')[1];
          resolve({
            type: 'binary',
            encoding: 'base64',
            data: base64,
            mimeType,
          });
        };
        reader.onerror = () => reject(new Error('Failed to read file'));
        reader.readAsDataURL(file);
      });
    },
    []
  );

  /**
   * Uploads a file to VFS at the desktop directory.
   * Handles file conflicts by checking if file exists.
   */
  const uploadFileToVFS = useCallback(
    async (file: File, dropCoordinates: { x: number; y: number }): Promise<boolean> => {
      try {
        // Validate file size
        if (file.size > MAX_FILE_SIZE_BYTES) {
          addToast({
            title: `File "${file.name}" is too large. Maximum size is 10MB.`,
            severity: 'error',
          });
          return false;
        }

        const vfs = getVFSService();

        // Check VFS connection
        if (!vfs.isInitialized()) {
          addToast({
            title: 'Storage disconnected. Cannot upload files.',
            severity: 'error',
          });
          return false;
        }

        const filePath = `${DESKTOP_DIRECTORY}/${file.name}`;

        // Check if file already exists
        try {
          await vfs.readFile(filePath);
          // File exists - ask user what to do
          const overwrite = confirm(`"${file.name}" already exists. Overwrite?`);
          if (!overwrite) {
            addToast({
              title: `Upload cancelled for "${file.name}"`,
              severity: 'warning',
            });
            return false;
          }
        } catch (_error) {
          // File doesn't exist, proceed with upload
        }

        // Read file content
        const content = await readFileContent(file);

        // Generate thumbnail for images and text files
        const mimeType = file.type || getMimeType(file.name);
        const fileName = file.name.toUpperCase();

        // Check if it's a known extensionless text file
        const isExtensionlessText = EXTENSIONLESS_TEXT_FILES.some(
          (name) => fileName === name || fileName === name.toLowerCase()
        );

        // Generate thumbnail
        let thumbnailDataUrl: string | undefined;
        if (mimeType.startsWith('image/')) {
          thumbnailDataUrl = (await generateImageThumbnail(file)) || undefined;
        } else if (
          mimeType.startsWith('text/') ||
          mimeType === 'application/json' ||
          mimeType === 'application/x-yaml' ||
          mimeType === 'application/yaml' ||
          mimeType === 'text/yaml' ||
          mimeType === 'text/x-yaml' ||
          isExtensionlessText
        ) {
          thumbnailDataUrl = (await generateTextThumbnail(file)) || undefined;
        }

        // If thumbnail was generated, store it in a separate file
        let thumbnailPath: string | undefined;
        if (thumbnailDataUrl) {
          // Extract fileId for thumbnail path
          const fileId = file.name.startsWith('.')
            ? file.name
            : file.name.replace(/\.[^.]+$/, '');
          thumbnailPath = `${THUMBNAILS_DIRECTORY}/${fileId}.png`;

          // Extract base64 data from data URL (format: data:image/png;base64,XXXX)
          const base64Data = thumbnailDataUrl.split(',')[1];

          // Write thumbnail to separate VFS file
          try {
            await vfs.writeFile(
              thumbnailPath,
              {
                content: {
                  data: base64Data,
                  mimeType: 'image/png',
                },
              },
              true // create=true
            );
            console.log('[useFileDrop] Thumbnail written to:', thumbnailPath);
          } catch (thumbnailError) {
            console.warn(
              '[useFileDrop] Failed to write thumbnail, continuing without:',
              thumbnailError
            );
            thumbnailPath = undefined;
          }
        }

        // Desktop metadata (store thumbnailPath instead of inline thumbnail)
        const desktopMeta = {
          mimeType,
          ...(thumbnailPath && { thumbnailPath }),
        };

        // Write to VFS - different methods for text vs binary
        console.log('[useFileDrop] Writing file to VFS:', filePath, 'with metadata:', desktopMeta);

        if (typeof content === 'string') {
          // Text file: store as text with metadata
          const docContent = {
            text: content,
            desktopMeta,
          };
          await vfs.writeFile(filePath, { content: docContent }, true); // create=true for new files
        } else {
          // Binary file: use writeFileWithBytes with metadata in content
          const docContent = {
            desktopMeta,
          };
          await vfs.writeFileWithBytes(filePath, docContent, content.data, true); // create=true for new files
        }

        console.log('[useFileDrop] File written successfully:', filePath);

        // Save position and notify service about new file
        const fileId = file.name.startsWith('.') ? file.name : file.name.replace(/\.[^.]+$/, '');
        const service = getDesktopService();

        // Set position first (so onFileAdded doesn't auto-position)
        service.setPosition(fileId, dropCoordinates.x, dropCoordinates.y);
        console.log('[useFileDrop] Position saved for new file:', fileId, dropCoordinates);

        // Load file and notify listeners (this will create the shape)
        await service.onFileAdded(filePath);

        return true;
      } catch (error) {
        console.error(`[useFileDrop] Failed to upload file ${file.name}:`, error);
        addToast({
          title: `Failed to upload "${file.name}". Check console for details.`,
          severity: 'error',
        });
        return false;
      }
    },
    [readFileContent, addToast]
  );

  /**
   * Handles file drop event.
   * Converts screen coordinates to canvas coordinates and uploads files.
   * CRITICAL: Must stop propagation to prevent TLDraw from handling the drop.
   */
  const handleDrop = useCallback(
    async (e: React.DragEvent<HTMLDivElement> | DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      // When called from native addEventListener, e is already the native event
      if ('stopImmediatePropagation' in e) {
        e.stopImmediatePropagation();
      }
      setIsDraggingOver(false);

      // Get dropped files
      if (!e.dataTransfer?.files) {
        return;
      }

      const files = Array.from(e.dataTransfer.files);

      if (files.length === 0) {
        return;
      }

      // Convert screen coordinates to canvas coordinates
      const screenPoint = { x: e.clientX, y: e.clientY };
      const canvasPoint = editor.screenToPage(screenPoint);

      console.log(
        `[useFileDrop] Dropping ${files.length} file(s) at canvas coordinates:`,
        canvasPoint
      );

      // Upload files sequentially to avoid race conditions
      let successCount = 0;
      let failCount = 0;

      // Show initial progress toast for multiple files
      let progressToastId: string | undefined;
      if (files.length > 1) {
        progressToastId = addToast({
          title: `Uploading ${files.length} files...`,
          description: `0 of ${files.length} completed`,
          severity: 'info',
          keepOpen: true,
        });
      }

      for (let i = 0; i < files.length; i++) {
        const file = files[i];

        // Offset each file slightly to avoid exact overlap
        const offsetX = i * 10;
        const offsetY = i * 10;
        const dropCoords = {
          x: canvasPoint.x + offsetX,
          y: canvasPoint.y + offsetY,
        };

        const success = await uploadFileToVFS(file, dropCoords);
        if (success) {
          successCount++;
        } else {
          failCount++;
        }

        // Update progress toast for multiple files
        if (progressToastId) {
          const completed = successCount + failCount;
          if (completed < files.length) {
            // Still uploading - update progress
            removeToast(progressToastId);
            progressToastId = addToast({
              title: `Uploading ${files.length} files...`,
              description: `${completed} of ${files.length} completed`,
              severity: 'info',
              keepOpen: true,
            });
          } else {
            // Done - remove progress toast
            removeToast(progressToastId);
          }
        }
      }

      // Show final result notification
      if (successCount > 0) {
        const message =
          successCount === 1
            ? `Uploaded "${files[0].name}"`
            : `Uploaded ${successCount} file${successCount > 1 ? 's' : ''}`;
        addToast({
          title: message,
          severity: 'success',
        });
      }

      if (failCount > 0 && successCount === 0) {
        addToast({
          title: `Failed to upload ${failCount} file${failCount > 1 ? 's' : ''}`,
          severity: 'error',
        });
      }
    },
    [editor, uploadFileToVFS, addToast, removeToast]
  );

  /**
   * Handles drag over event to enable drop and show visual feedback.
   */
  const handleDragOver = useCallback((e: React.DragEvent<HTMLDivElement> | DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    // Check if dragging files (not text or other data)
    const hasFiles = e.dataTransfer?.types.includes('Files');
    if (hasFiles && e.dataTransfer) {
      e.dataTransfer.dropEffect = 'copy';
      setIsDraggingOver(true);
    }
  }, []);

  /**
   * Handles drag enter event.
   */
  const handleDragEnter = useCallback((e: React.DragEvent<HTMLDivElement> | DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const hasFiles = e.dataTransfer?.types.includes('Files');
    if (hasFiles) {
      setIsDraggingOver(true);
    }
  }, []);

  /**
   * Handles drag leave event to remove visual feedback.
   */
  const handleDragLeave = useCallback((e: React.DragEvent<HTMLDivElement> | DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    // Only remove highlight if we're leaving the drop zone completely
    // (not just entering a child element)
    const target = (e.currentTarget as HTMLElement) || (e.target as HTMLElement);
    const rect = target.getBoundingClientRect();
    const x = e.clientX;
    const y = e.clientY;

    if (x < rect.left || x >= rect.right || y < rect.top || y >= rect.bottom) {
      setIsDraggingOver(false);
    }
  }, []);

  return {
    isDraggingOver,
    handleDrop,
    handleDragOver,
    handleDragEnter,
    handleDragLeave,
  };
}

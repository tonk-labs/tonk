import { useCallback, useState } from 'react';
import { useEditor } from 'tldraw';
import { getVFSService } from '../../../lib/vfs-service';
import { showError, showSuccess, showWarning } from '../../../lib/notifications';
import { DESKTOP_DIRECTORY } from './useDesktopSync';
import { getMimeType } from '../utils/mimeResolver';

/**
 * Maximum file size allowed for upload (10MB).
 * Prevents browser memory issues with very large files.
 */
const MAX_FILE_SIZE_BYTES = 10 * 1024 * 1024;

/**
 * Hook that handles drag and drop of files onto the desktop canvas.
 * Uploads files to VFS and creates file-icon shapes at drop coordinates.
 */
export function useFileDrop() {
  const editor = useEditor();
  const [isDraggingOver, setIsDraggingOver] = useState(false);

  /**
   * Reads a file's content as text or base64 depending on file type.
   */
  const readFileContent = useCallback(async (file: File): Promise<string | object> => {
    const mimeType = file.type || getMimeType(file.name);

    // For text files, read as text
    if (mimeType.startsWith('text/') || mimeType === 'application/json') {
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
  }, []);

  /**
   * Uploads a file to VFS at the desktop directory.
   * Handles file conflicts by checking if file exists.
   */
  const uploadFileToVFS = useCallback(async (
    file: File,
    dropCoordinates: { x: number; y: number }
  ): Promise<boolean> => {
    try {
      // Validate file size
      if (file.size > MAX_FILE_SIZE_BYTES) {
        showError(`File "${file.name}" is too large. Maximum size is 10MB.`);
        return false;
      }

      const vfs = getVFSService();

      // Check VFS connection
      if (!vfs.isInitialized()) {
        showError('Storage disconnected. Cannot upload files.');
        return false;
      }

      const filePath = `${DESKTOP_DIRECTORY}/${file.name}`;

      // Check if file already exists
      try {
        await vfs.readFile(filePath);
        // File exists - ask user what to do
        const overwrite = confirm(`"${file.name}" already exists. Overwrite?`);
        if (!overwrite) {
          showWarning(`Upload cancelled for "${file.name}"`);
          return false;
        }
      } catch (error) {
        // File doesn't exist, proceed with upload
      }

      // Read file content
      const content = await readFileContent(file);

      // Create document with desktop metadata (position)
      const doc = {
        content: typeof content === 'string' ? { text: content } : content,
        desktopMeta: {
          x: dropCoordinates.x,
          y: dropCoordinates.y,
          mimeType: file.type || getMimeType(file.name),
        },
      };

      // Write to VFS
      await vfs.writeFile(filePath, { content: doc });

      return true;
    } catch (error) {
      console.error(`[useFileDrop] Failed to upload file ${file.name}:`, error);
      showError(`Failed to upload "${file.name}". Check console for details.`);
      return false;
    }
  }, [readFileContent]);

  /**
   * Handles file drop event.
   * Converts screen coordinates to canvas coordinates and uploads files.
   * CRITICAL: Must stop propagation to prevent TLDraw from handling the drop.
   */
  const handleDrop = useCallback(async (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    e.nativeEvent.stopImmediatePropagation(); // Stop TLDraw from seeing this event
    setIsDraggingOver(false);

    // Get dropped files
    const files = Array.from(e.dataTransfer.files);

    if (files.length === 0) {
      return;
    }

    // Convert screen coordinates to canvas coordinates
    const screenPoint = { x: e.clientX, y: e.clientY };
    const canvasPoint = editor.screenToPage(screenPoint);

    console.log(`[useFileDrop] Dropping ${files.length} file(s) at canvas coordinates:`, canvasPoint);

    // Upload files sequentially to avoid race conditions
    let successCount = 0;
    let failCount = 0;

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
    }

    // Show summary notification
    if (successCount > 0) {
      const message = successCount === 1
        ? `Uploaded "${files[0].name}"`
        : `Uploaded ${successCount} file${successCount > 1 ? 's' : ''}`;
      showSuccess(message);
    }

    if (failCount > 0 && successCount === 0) {
      showError(`Failed to upload ${failCount} file${failCount > 1 ? 's' : ''}`);
    }
  }, [editor, uploadFileToVFS]);

  /**
   * Handles drag over event to enable drop and show visual feedback.
   */
  const handleDragOver = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();

    // Check if dragging files (not text or other data)
    const hasFiles = e.dataTransfer.types.includes('Files');
    if (hasFiles) {
      e.dataTransfer.dropEffect = 'copy';
      setIsDraggingOver(true);
    }
  }, []);

  /**
   * Handles drag enter event.
   */
  const handleDragEnter = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    const hasFiles = e.dataTransfer.types.includes('Files');
    if (hasFiles) {
      setIsDraggingOver(true);
    }
  }, []);

  /**
   * Handles drag leave event to remove visual feedback.
   */
  const handleDragLeave = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();

    // Only remove highlight if we're leaving the drop zone completely
    // (not just entering a child element)
    const rect = e.currentTarget.getBoundingClientRect();
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

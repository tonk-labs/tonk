import type { JSONContent } from '@tiptap/react';
import { useCallback, useEffect, useRef } from 'react';
import { getDesktopService, invalidateThumbnailCache } from '@/features/desktop';
import { THUMBNAILS_DIRECTORY } from '@/features/desktop/constants';
import type { DesktopFile } from '@/features/desktop/types';
import { generateTextThumbnailFromContent } from '@/features/desktop/utils/thumbnailGenerator';
import { useEditorStore } from '@/features/editor/stores/editorStore';
import type { JsonValue, VFSService } from '@/vfs-client';

type DesktopMeta = DesktopFile['desktopMeta'];

interface UseEditorVFSSaveOptions {
  filePath: string | null;
  vfs: VFSService;
  debounceMs?: number;
}

/**
 * Generate and save thumbnail for a file.
 * Called once when leaving the editor (on unmount).
 */
async function generateThumbnail(text: string, filePath: string, vfs: VFSService): Promise<void> {
  const fileName = filePath.split('/').pop() || 'file.txt';
  const fileId = fileName.startsWith('.') ? fileName : fileName.replace(/\.[^.]+$/, '');

  console.log('[EditorVFSSave] Generating thumbnail on unmount for:', fileName);

  const thumbnailDataUrl = await generateTextThumbnailFromContent(text, fileName);
  if (!thumbnailDataUrl) return;

  const thumbnailPath = `${THUMBNAILS_DIRECTORY}/${fileId}.png`;
  const base64Data = thumbnailDataUrl.split(',')[1];

  // Write thumbnail file
  const thumbnailExists = await vfs.exists(thumbnailPath);
  await vfs.writeFile(
    thumbnailPath,
    {
      content: {
        data: base64Data,
        mimeType: 'image/png',
      },
    },
    !thumbnailExists
  );

  // Update file with thumbnailPath and version - preserve existing content
  const currentDoc = await vfs.readFile(filePath);
  const currentContent = currentDoc.content as {
    desktopMeta?: DesktopMeta;
    [key: string]: unknown;
  };
  const currentDesktopMeta = currentContent?.desktopMeta || {};

  // Use updateFile with merged content including updated desktopMeta
  await vfs.updateFile(filePath, {
    ...currentContent,
    desktopMeta: {
      ...currentDesktopMeta,
      thumbnailPath,
      thumbnailVersion: Date.now(),
    },
  } as JsonValue);

  // Invalidate cache and notify desktop
  invalidateThumbnailCache(thumbnailPath);
  const desktopService = getDesktopService();
  await desktopService.reloadFile(filePath);

  console.log('[EditorVFSSave] ✅ Thumbnail generated on unmount:', thumbnailPath);
}

/**
 * Hook that auto-saves editor content to VFS when changes occur.
 * Debounces writes to avoid excessive VFS operations.
 */
export function useEditorVFSSave({ filePath, vfs, debounceMs = 1000 }: UseEditorVFSSaveOptions) {
  const saveTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastSavedContentRef = useRef<string | null>(null);
  const lastFilePathRef = useRef<string | null>(null);

  // Initialize lastSavedContentRef with current content when file opens
  // This prevents unnecessary saves when closing without changes
  useEffect(() => {
    // Reset when file changes
    if (filePath !== lastFilePathRef.current) {
      lastFilePathRef.current = filePath;
      const state = useEditorStore.getState();

      // Use appropriate content source based on file type
      if (state.isMarkdownFile && state.rawMarkdownContent) {
        lastSavedContentRef.current = state.rawMarkdownContent;
      } else if (state.document) {
        lastSavedContentRef.current = jsonContentToText(state.document);
      } else {
        lastSavedContentRef.current = null;
      }
    }
  }, [filePath]);

  const saveToVFS = useCallback(
    async (content: JSONContent) => {
      if (!filePath || !vfs) return;

      try {
        const state = useEditorStore.getState();

        // Get text based on file type
        let text: string;
        if (state.isMarkdownFile && state.rawMarkdownContent) {
          // For markdown files: use the markdown representation from the editor
          text = state.rawMarkdownContent;
        } else {
          // For other files: convert JSON to plain text
          text = jsonContentToText(content);
        }

        // Skip if content hasn't changed
        if (text === lastSavedContentRef.current) return;

        // Read existing file to preserve desktopMeta
        let existingDesktopMeta: DesktopMeta | undefined;
        try {
          const existingDoc = await vfs.readFile(filePath);
          const existingContent = existingDoc.content as { desktopMeta?: DesktopMeta };
          if (existingContent?.desktopMeta && typeof existingContent.desktopMeta === 'object') {
            existingDesktopMeta = existingContent.desktopMeta;
          }
        } catch {
          // File might not exist yet, that's fine
        }

        // Use updateFile with merged content
        await vfs.updateFile(filePath, {
          text,
          ...(existingDesktopMeta && { desktopMeta: existingDesktopMeta }),
        } as JsonValue);

        lastSavedContentRef.current = text;
      } catch (error) {
        console.warn(`Failed to save editor content to ${filePath}:`, error);
      }
    },
    [filePath, vfs]
  );

  useEffect(() => {
    if (!filePath || !vfs) return;

    // Subscribe to editor store changes
    const unsubscribe = useEditorStore.subscribe((state) => {
      if (!state.document) return;

      // Clear pending save
      if (saveTimeoutRef.current) {
        clearTimeout(saveTimeoutRef.current);
      }

      // Debounce the save
      saveTimeoutRef.current = setTimeout(() => {
        if (state.document) {
          saveToVFS(state.document);
        }
      }, debounceMs);
    });

    // Cleanup on unmount - save content and generate thumbnail
    return () => {
      unsubscribe();

      // Clear pending save timeout
      if (saveTimeoutRef.current) {
        clearTimeout(saveTimeoutRef.current);
      }

      // Final save with current content and generate thumbnail
      const currentState = useEditorStore.getState();
      if (currentState.document && filePath) {
        // Get correct text for thumbnail based on file type
        const text =
          currentState.isMarkdownFile && currentState.rawMarkdownContent
            ? currentState.rawMarkdownContent
            : jsonContentToText(currentState.document);

        // Save content first (sync-ish via fire-and-forget)
        saveToVFS(currentState.document);

        // Generate thumbnail (fire-and-forget, will complete after navigation)
        generateThumbnail(text, filePath, vfs).catch((error) => {
          console.warn('[EditorVFSSave] Failed to generate thumbnail on unmount:', error);
        });
      }
    };
  }, [filePath, vfs, debounceMs, saveToVFS]);
}

/**
 * Convert Tiptap JSONContent to plain text.
 * Preserves line breaks between paragraphs.
 */
function jsonContentToText(content: JSONContent): string {
  if (!content.content) return '';

  return content.content
    .map((node) => {
      if (node.type === 'paragraph') {
        if (!node.content) return '';
        return node.content
          .map((child) => {
            if (child.type === 'text') return child.text || '';
            return '';
          })
          .join('');
      }
      // Handle other node types as needed
      if (node.type === 'heading' && node.content) {
        return node.content
          .map((child) => (child.type === 'text' ? child.text || '' : ''))
          .join('');
      }
      return '';
    })
    .join('\n');
}

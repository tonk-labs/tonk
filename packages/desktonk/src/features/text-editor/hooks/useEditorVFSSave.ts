import type { JSONContent } from '@tiptap/react';
import type { VFSService } from '@/vfs-client';
import { useCallback, useEffect, useRef } from 'react';
import { getDesktopService } from '@/features/desktop';
import { generateTextThumbnailFromContent } from '@/features/desktop/utils/thumbnailGenerator';
import { useEditorStore } from '@/features/editor/stores/editorStore';

interface UseEditorVFSSaveOptions {
  filePath: string | null;
  vfs: VFSService;
  debounceMs?: number;
}

/**
 * Hook that auto-saves editor content to VFS when changes occur.
 * Debounces writes to avoid excessive VFS operations.
 */
export function useEditorVFSSave({
  filePath,
  vfs,
  debounceMs = 1000,
}: UseEditorVFSSaveOptions) {
  const saveTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const thumbnailTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
    null
  );
  const lastSavedContentRef = useRef<string | null>(null);
  const lastFilePathRef = useRef<string | null>(null);

  // Initialize lastSavedContentRef with current content when file opens
  // This prevents unnecessary saves when closing without changes
  useEffect(() => {
    // Reset when file changes
    if (filePath !== lastFilePathRef.current) {
      lastFilePathRef.current = filePath;
      const currentDocument = useEditorStore.getState().document;
      if (currentDocument) {
        lastSavedContentRef.current = jsonContentToText(currentDocument);
      } else {
        lastSavedContentRef.current = null;
      }
    }
  }, [filePath]);

  const saveToVFS = useCallback(
    async (content: JSONContent) => {
      if (!filePath || !vfs) return;

      try {
        // Convert JSONContent back to plain text for storage
        const text = jsonContentToText(content);

        // Skip if content hasn't changed
        if (text === lastSavedContentRef.current) return;

        await vfs.writeFile(filePath, {
          content: { text },
        });

        lastSavedContentRef.current = text;

        // Schedule thumbnail regeneration (more debounced since it's expensive)
        if (thumbnailTimeoutRef.current) {
          clearTimeout(thumbnailTimeoutRef.current);
        }

        thumbnailTimeoutRef.current = setTimeout(async () => {
          try {
            // Extract file name from path
            const fileName = filePath.split('/').pop() || 'file.txt';
            console.log(
              '[EditorVFSSave] Starting thumbnail regeneration for:',
              fileName
            );

            // Generate new thumbnail
            const thumbnail = await generateTextThumbnailFromContent(
              text,
              fileName
            );
            console.log(
              '[EditorVFSSave] Thumbnail generated:',
              thumbnail ? 'YES' : 'NO'
            );

            if (thumbnail) {
              // Use patchFile to update only the thumbnail field
              await vfs.patchFile(filePath, ['desktopMeta', 'thumbnail'], thumbnail);
              console.log('[EditorVFSSave] ✅ Patched thumbnail');

              // Notify desktop service to refresh the file icon
              const desktopService = getDesktopService();
              console.log('[EditorVFSSave] Calling reloadFile:', filePath);
              await desktopService.reloadFile(filePath);
              console.log('[EditorVFSSave] reloadFile complete');
            }
          } catch (error) {
            console.warn(
              `Failed to regenerate thumbnail for ${filePath}:`,
              error
            );
          }
        }, 2000); // 2 second debounce for thumbnail regeneration
      } catch (error) {
        console.warn(`Failed to save editor content to ${filePath}:`, error);
      }
    },
    [filePath, vfs]
  );

  useEffect(() => {
    if (!filePath || !vfs) return;

    // Subscribe to editor store changes
    const unsubscribe = useEditorStore.subscribe(state => {
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

    // Cleanup on unmount - save immediately to prevent data loss
    return () => {
      unsubscribe();

      // Clear pending timeouts
      if (saveTimeoutRef.current) {
        clearTimeout(saveTimeoutRef.current);
      }
      if (thumbnailTimeoutRef.current) {
        clearTimeout(thumbnailTimeoutRef.current);
      }

      // Final save with current content (don't wait for debounce)
      const currentDocument = useEditorStore.getState().document;
      if (currentDocument) {
        saveToVFS(currentDocument);
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
    .map(node => {
      if (node.type === 'paragraph') {
        if (!node.content) return '';
        return node.content
          .map(child => {
            if (child.type === 'text') return child.text || '';
            return '';
          })
          .join('');
      }
      // Handle other node types as needed
      if (node.type === 'heading' && node.content) {
        return node.content
          .map(child => (child.type === 'text' ? child.text || '' : ''))
          .join('');
      }
      return '';
    })
    .join('\n');
}

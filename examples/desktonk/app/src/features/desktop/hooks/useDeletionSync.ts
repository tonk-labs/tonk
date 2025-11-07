import { useEffect } from 'react';
import { useEditor } from 'tldraw';
import type { FileIconShape } from '../shapes';
import { getVFSService } from '../../../lib/vfs-service';
import { showError } from '../../../lib/notifications';

export function useDeletionSync() {
  const editor = useEditor();

  useEffect(() => {
    const vfs = getVFSService();

    const unsubscribe = editor.store.listen(
      (change) => {
        const removedShapes = Object.values(change.changes.removed);
        
        for (const shape of removedShapes) {
          if (shape.typeName === 'shape' && shape.type === 'file-icon') {
            const fileIconShape = shape as FileIconShape;
            const filePath = fileIconShape.props.filePath;

            if (!filePath) {
              console.warn('[useDeletionSync] Skipping deletion for shape with empty filePath');
              continue;
            }

            vfs.deleteFile(filePath).catch((err) => {
              console.error('[useDeletionSync] Failed to delete file', err);
              showError(`Failed to delete ${fileIconShape.props.fileName}: ${err.message}`);
            });
          }
        }
      },
      { source: 'user', scope: 'document' }
    );

    return () => {
      unsubscribe();
    };
  }, [editor]);
}

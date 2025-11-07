import { useEffect, useRef } from 'react';
import { useEditor } from 'tldraw';
import type { FileIconShape } from '../shapes';
import { getVFSService } from '../../../lib/vfs-service';
import { syncCoordinator } from './syncCoordinator';

/**
 * Hook that listens for FileIcon shape position changes and persists them to VFS.
 * Uses debouncing to avoid excessive VFS writes during dragging operations.
 */
export function usePositionSync() {
  const editor = useEditor();
  const saveTimeoutRef = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const previousPositionsRef = useRef<Record<string, { x: number; y: number }>>({});

  useEffect(() => {
    // Listen to all changes in the editor's store
    const unsubscribe = editor.store.listen(
      (change) => {
        // Process added and updated records
        const shapes = [
          ...Object.values(change.changes.added),
          ...Object.values(change.changes.updated).map(([_prev, next]) => next),
        ];

        for (const shape of shapes) {
          // Only process file-icon shapes
          if (shape.typeName !== 'shape' || shape.type !== 'file-icon') {
            continue;
          }

          const fileIconShape = shape as FileIconShape;
          const shapeId = fileIconShape.id;

          // Check if position actually changed
          const previousPosition = previousPositionsRef.current[shapeId];
          const currentPosition = { x: fileIconShape.x, y: fileIconShape.y };

          if (
            previousPosition &&
            previousPosition.x === currentPosition.x &&
            previousPosition.y === currentPosition.y
          ) {
            // Position hasn't changed, skip
            continue;
          }

          // Update tracked position
          previousPositionsRef.current[shapeId] = currentPosition;

          // Clear any pending save for this shape
          if (saveTimeoutRef.current[shapeId]) {
            clearTimeout(saveTimeoutRef.current[shapeId]);
            syncCoordinator.unregisterPendingSave(shapeId);
          }

          // Debounce the save operation (500ms)
          const timeout = setTimeout(() => {
            syncCoordinator.unregisterPendingSave(shapeId);
            savePosition(fileIconShape);
            delete saveTimeoutRef.current[shapeId];
          }, 500);

          saveTimeoutRef.current[shapeId] = timeout;
          syncCoordinator.registerPendingSave(shapeId, timeout);
        }
      },
      { source: 'user', scope: 'document' }
    );

    // Cleanup function
    return () => {
      unsubscribe();
      // Clear all pending timeouts
      Object.values(saveTimeoutRef.current).forEach(clearTimeout);
      saveTimeoutRef.current = {};
    };
  }, [editor]);
}

/**
 * Saves the position of a FileIcon shape to VFS by updating its desktopMeta.
 * Uses syncCoordinator to prevent triggering infinite reload loops.
 */
async function savePosition(shape: FileIconShape): Promise<void> {
  const shapeId = shape.id;

  try {
    // Mark save as in-progress BEFORE writing to VFS
    syncCoordinator.startPositionSave(shapeId);

    const vfs = getVFSService();
    const doc = await vfs.readFile(shape.props.filePath);
    const content = doc.content as Record<string, unknown>;

    const updatedContent = {
      ...content,
      desktopMeta: {
        ...(content?.desktopMeta as Record<string, unknown> | undefined),
        x: shape.x,
        y: shape.y,
      },
    };

    await vfs.writeFile(shape.props.filePath, { content: updatedContent });
    console.log(`Saved position for ${shape.props.fileName}:`, { x: shape.x, y: shape.y });
  } catch (error) {
    console.error('Failed to save position:', error);
  } finally {
    // Always mark save as complete, even if it failed
    syncCoordinator.endPositionSave(shapeId);
  }
}

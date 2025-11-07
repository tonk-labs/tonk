import { useEffect, useRef } from 'react';
import { useEditor } from 'tldraw';
import type { FileIconShape } from '../shapes';
import { getVFSService } from '../../../lib/vfs-service';
import { syncCoordinator } from './syncCoordinator';
import { showError, showWarning } from '../../../lib/notifications';

/**
 * Map of file paths to their in-progress save operations.
 * Used to serialize concurrent saves to the same file.
 */
type SaveOperationMap = Map<string, Promise<void>>;

/**
 * Debounce delay for position saves (in milliseconds).
 * Balances between responsiveness and reducing VFS write frequency.
 * Too short: excessive VFS writes during dragging.
 * Too long: delayed persistence, poor user experience.
 */
const POSITION_SAVE_DEBOUNCE_MS = 500;

/**
 * Duration for error notifications about position save failures (in milliseconds).
 * Longer than default since position persistence failure is important.
 */
const ERROR_NOTIFICATION_DURATION_MS = 7000;

/**
 * Hook that listens for FileIcon shape position changes and persists them to VFS.
 * Uses debouncing to avoid excessive VFS writes during dragging operations.
 */
export function usePositionSync() {
  const editor = useEditor();
  const saveTimeoutRef = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const previousPositionsRef = useRef<Record<string, { x: number; y: number }>>({});
  const saveInProgressRef = useRef<SaveOperationMap>(new Map());

  useEffect(() => {
    // Listen to all changes in the editor's store
    const unsubscribe = editor.store.listen(
      (change) => {
        // Clean up removed shapes to prevent memory leak
        const removedShapes = Object.values(change.changes.removed);
        for (const shape of removedShapes) {
          if (shape.typeName === 'shape' && shape.type === 'file-icon') {
            const shapeId = shape.id;
            // Remove from position tracking
            delete previousPositionsRef.current[shapeId];
            // Cancel any pending save
            if (saveTimeoutRef.current[shapeId]) {
              clearTimeout(saveTimeoutRef.current[shapeId]);
              delete saveTimeoutRef.current[shapeId];
              syncCoordinator.unregisterPendingSave(shapeId);
            }
          }
        }

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

          // Debounce the save operation
          const timeout = setTimeout(() => {
            syncCoordinator.unregisterPendingSave(shapeId);
            savePosition(editor, fileIconShape, saveInProgressRef.current);
            delete saveTimeoutRef.current[shapeId];
          }, POSITION_SAVE_DEBOUNCE_MS);

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
      // Clear position tracking to prevent memory leak
      previousPositionsRef.current = {};
    };
  }, [editor]);
}

/**
 * Saves the position of a FileIcon shape to VFS by updating its desktopMeta.
 * Uses syncCoordinator to prevent triggering infinite reload loops.
 * Validates that the shape still exists before writing to prevent race conditions.
 * Uses per-file mutex to prevent concurrent saves from corrupting data.
 */
async function savePosition(
  editor: ReturnType<typeof useEditor>,
  shape: FileIconShape,
  saveInProgress: SaveOperationMap
): Promise<void> {
  const shapeId = shape.id;
  const filePath = shape.props.filePath;

  // Validate editor is still available
  if (!editor || !editor.getShape) {
    console.warn('[usePositionSync] Editor unavailable, skipping position save');
    return;
  }

  // Wait for any in-progress save to this file to complete
  // This prevents read-modify-write race conditions
  const pendingSave = saveInProgress.get(filePath);
  if (pendingSave) {
    console.log(`[usePositionSync] Waiting for in-progress save to ${filePath}`);
    await pendingSave;
  }

  // Create promise for this save operation
  const savePromise = (async () => {
    try {
      // Mark save as in-progress BEFORE writing to VFS
      syncCoordinator.startPositionSave(shapeId);

      // Validate that the shape still exists before starting the write
      // If it was deleted during the debounce delay, skip the save
      const currentShape = editor.getShape(shapeId) as FileIconShape | undefined;
      if (!currentShape || currentShape.type !== 'file-icon') {
        console.log(`[usePositionSync] Shape ${shapeId} no longer exists, skipping position save`);
        return;
      }

      // Use the current shape's position (not the captured one from the closure)
      // This ensures we save the most up-to-date position
      const positionToSave = { x: currentShape.x, y: currentShape.y };

      const vfs = getVFSService();

      // Check if VFS is connected before attempting save
      if (!vfs.isInitialized()) {
        console.warn(`[usePositionSync] VFS disconnected, cannot save position for ${shape.props.fileName}`);
        showWarning('Storage disconnected. Icon positions cannot be saved until reconnected.');
        return;
      }

      const doc = await vfs.readFile(filePath);
      const content = doc.content as Record<string, unknown>;

      // Validate shape still exists after async read (could have been deleted)
      const shapeAfterRead = editor.getShape(shapeId) as FileIconShape | undefined;
      if (!shapeAfterRead || shapeAfterRead.type !== 'file-icon') {
        console.log(`[usePositionSync] Shape ${shapeId} was deleted during read, skipping position save`);
        return;
      }

      // Validate position hasn't changed during read (concurrent drag)
      // If position changed, we need to re-queue a save for the new position
      if (shapeAfterRead.x !== positionToSave.x || shapeAfterRead.y !== positionToSave.y) {
        console.log(
          `[usePositionSync] Position changed during save (${positionToSave.x},${positionToSave.y} → ${shapeAfterRead.x},${shapeAfterRead.y}). ` +
            'Skipping this save - position tracking will trigger a new save for the current position.'
        );
        return;
      }

      const updatedContent = {
        ...content,
        desktopMeta: {
          ...(content?.desktopMeta as Record<string, unknown> | undefined),
          x: positionToSave.x,
          y: positionToSave.y,
        },
      };

      await vfs.writeFile(filePath, { content: updatedContent });
      console.log(`[usePositionSync] Saved position for ${shape.props.fileName}:`, positionToSave);
    } catch (error) {
      console.error(`[usePositionSync] CRITICAL: Failed to save position for ${shape.props.fileName}:`, error);
      // Show user-facing notification - this is a data loss scenario
      showError(
        `Failed to save position for "${shape.props.fileName}". Your icon arrangement may not persist.`,
        ERROR_NOTIFICATION_DURATION_MS
      );
    } finally {
      // Always mark save as complete, even if it failed
      syncCoordinator.endPositionSave(shapeId);
    }
  })();

  // Register this save operation
  saveInProgress.set(filePath, savePromise);

  try {
    await savePromise;
  } finally {
    // Clean up completed save
    saveInProgress.delete(filePath);
  }
}

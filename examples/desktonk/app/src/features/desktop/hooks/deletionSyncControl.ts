/**
 * Global control to temporarily pause deletion sync during desktop file reload.
 * Prevents useDeletionSync from deleting VFS files when shapes are cleared for rebuild.
 */
let isPaused = false;

export const deletionSyncControl = {
  pause: () => {
    isPaused = true;
    console.log('[deletionSyncControl] Deletion sync paused');
  },
  resume: () => {
    isPaused = false;
    console.log('[deletionSyncControl] Deletion sync resumed');
  },
  isPaused: () => isPaused,
};

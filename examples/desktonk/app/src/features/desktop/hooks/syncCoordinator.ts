/**
 * Synchronization coordinator to prevent infinite loops between
 * position saves (usePositionSync) and directory watching (useDesktopSync).
 *
 * The problem: Position saves trigger VFS writes, which trigger directory
 * watcher callbacks, which reload all files and recreate shapes, which
 * trigger more position changes, causing an infinite loop.
 *
 * The solution: Track when position saves are in-flight and skip directory
 * reload during those operations.
 *
 * CRITICAL #4 Fix: Also tracks pending (debounced) position save timeouts
 * so they can be canceled when shapes are recreated during directory reload.
 * This prevents stale shape references from corrupting VFS data.
 */

class SyncCoordinator {
  private inProgressSaves = new Set<string>();
  private pendingSaveTimeouts = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly GRACE_PERIOD_MS = 100;

  /**
   * Mark the start of a position save operation.
   * @param shapeId - Unique identifier for the shape being saved
   */
  startPositionSave(shapeId: string): void {
    this.inProgressSaves.add(shapeId);
    console.debug(`[SyncCoordinator] Started position save for ${shapeId}. In-flight: ${this.inProgressSaves.size}`);
  }

  /**
   * Mark the end of a position save operation.
   * Includes a grace period to handle async timing issues.
   * @param shapeId - Unique identifier for the shape that was saved
   */
  endPositionSave(shapeId: string): void {
    // Add a small grace period to ensure watcher events have fired
    setTimeout(() => {
      this.inProgressSaves.delete(shapeId);
      console.debug(`[SyncCoordinator] Ended position save for ${shapeId}. In-flight: ${this.inProgressSaves.size}`);
    }, this.GRACE_PERIOD_MS);
  }

  /**
   * Check if a directory reload should be skipped because position saves
   * are currently in progress.
   * @returns true if reload should be skipped, false otherwise
   */
  shouldSkipReload(): boolean {
    const hasInProgressSaves = this.inProgressSaves.size > 0;
    if (hasInProgressSaves) {
      console.debug(`[SyncCoordinator] Skipping reload - ${this.inProgressSaves.size} position save(s) in progress`);
    }
    return hasInProgressSaves;
  }

  /**
   * Get the count of in-progress saves (for debugging/testing)
   */
  getInProgressCount(): number {
    return this.inProgressSaves.size;
  }

  /**
   * Clear all tracked saves (for testing/cleanup)
   */
  reset(): void {
    this.inProgressSaves.clear();
    console.debug('[SyncCoordinator] Reset all in-progress saves');
  }

  /**
   * Register a pending position save timeout.
   * This allows us to cancel it later if the shape is recreated.
   * @param shapeId - Unique identifier for the shape
   * @param timeout - The timeout handle to track
   */
  registerPendingSave(shapeId: string, timeout: ReturnType<typeof setTimeout>): void {
    // Cancel any existing timeout for this shape
    const existingTimeout = this.pendingSaveTimeouts.get(shapeId);
    if (existingTimeout) {
      clearTimeout(existingTimeout);
    }
    this.pendingSaveTimeouts.set(shapeId, timeout);
    console.debug(`[SyncCoordinator] Registered pending save for ${shapeId}. Pending: ${this.pendingSaveTimeouts.size}`);
  }

  /**
   * Unregister a pending position save timeout (called when it fires).
   * @param shapeId - Unique identifier for the shape
   */
  unregisterPendingSave(shapeId: string): void {
    this.pendingSaveTimeouts.delete(shapeId);
    console.debug(`[SyncCoordinator] Unregistered pending save for ${shapeId}. Pending: ${this.pendingSaveTimeouts.size}`);
  }

  /**
   * Cancel all pending position save timeouts.
   * CRITICAL: This must be called BEFORE clearing shapes in loadDesktopFiles()
   * to prevent stale shape references from corrupting VFS data.
   *
   * Race condition prevented:
   * 1. User drags shape A from (100,100) to (200,200)
   * 2. 500ms debounce timer starts with reference to shape A
   * 3. Directory watcher fires, deletes shape A, creates shape B at (100,100)
   * 4. Debounce fires with STALE shape A reference, writes (200,200) to VFS
   * 5. Shape B shows (100,100) but VFS has (200,200) - DATA CORRUPTION
   *
   * By canceling pending saves before recreation, we prevent step 4.
   */
  cancelAllPendingSaves(): void {
    const count = this.pendingSaveTimeouts.size;
    this.pendingSaveTimeouts.forEach((timeout) => {
      clearTimeout(timeout);
    });
    this.pendingSaveTimeouts.clear();
    if (count > 0) {
      console.log(`[SyncCoordinator] Canceled ${count} pending position save(s) to prevent race condition`);
    }
  }

  /**
   * Get the count of pending saves (for debugging/testing)
   */
  getPendingSaveCount(): number {
    return this.pendingSaveTimeouts.size;
  }
}

// Singleton instance shared across both hooks
export const syncCoordinator = new SyncCoordinator();

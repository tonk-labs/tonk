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
 */

class SyncCoordinator {
  private inProgressSaves = new Set<string>();
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
}

// Singleton instance shared across both hooks
export const syncCoordinator = new SyncCoordinator();

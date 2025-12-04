import { getVFSService } from '@tonk/host-web/client';
import { DESKTOP_DIRECTORY, LAYOUT_DIRECTORY } from '../constants';
import type { DesktopFile } from '../types';
import {
  extractDesktopFile,
  getNextAutoLayoutPosition,
} from '../utils/fileMetadata';

/**
 * Position data stored per file.
 */
interface PositionData {
  x: number;
  y: number;
  fileId: string;
  filePath: string;
  updatedAt: number;
  [key: string]: string | number; // Index signature for JsonValue compatibility
}

/**
 * Desktop state exposed to consumers.
 */
export interface DesktopState {
  files: DesktopFile[];
  positions: Map<string, { x: number; y: number }>;
  isLoading: boolean;
}

type StateListener = (state: DesktopState) => void;

/**
 * Central service managing desktop files and their positions.
 *
 * Responsibilities:
 * - Watch user files directory (/desktonk)
 * - Watch layout directory (/var/lib/desktonk/layout)
 * - Maintain in-memory state of files and positions
 * - Provide reactive updates to UI
 * - Handle position persistence (isolated per file)
 *
 * Eliminates: syncCoordinator, usePositionSync, useDesktopSync,
 * useDeletionSync, deletionSyncControl, and all coordination logic.
 */
export class DesktopService {
  private files = new Map<string, DesktopFile>();
  private positions = new Map<string, { x: number; y: number }>();
  private listeners = new Set<StateListener>();
  private isLoading = true;
  private initialized = false;

  // VFS watcher IDs
  private filesWatcherId: string | null = null;
  private layoutWatcherId: string | null = null;
  private positionFileWatchers = new Map<string, string>(); // fileId -> watchId

  // Debounce state for position updates
  private pendingSaves = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly POSITION_SAVE_DEBOUNCE_MS = 500;

  /**
   * Initialize the service and start watching.
   */
  async initialize(): Promise<void> {
    if (this.initialized) {
      console.warn('[DesktopService] Already initialized');
      return;
    }

    const vfs = getVFSService();

    if (!vfs.isInitialized()) {
      throw new Error('VFS must be initialized before DesktopService');
    }

    console.log('[DesktopService] Initializing...');

    try {
      // Ensure directories exist
      await this.ensureDirectories();

      // Load initial state
      await this.loadFiles();
      await this.loadPositions();

      // Start watching
      await this.startWatching();

      this.initialized = true;
      this.isLoading = false;
      this.notifyListeners();

      console.log(
        '[DesktopService] Initialized with',
        this.files.size,
        'files'
      );
    } catch (error) {
      console.error('[DesktopService] Initialization failed:', error);
      this.isLoading = false;
      throw error;
    }
  }

  /**
   * Subscribe to state changes.
   */
  subscribe(listener: StateListener): () => void {
    this.listeners.add(listener);
    // Immediately notify with current state
    listener(this.getState());

    return () => {
      this.listeners.delete(listener);
    };
  }

  /**
   * Get current state snapshot.
   */
  getState(): DesktopState {
    return {
      files: Array.from(this.files.values()),
      positions: new Map(this.positions),
      isLoading: this.isLoading,
    };
  }

  /**
   * Set position for a file (debounced).
   * Creates/updates the position file in layout directory.
   */
  setPosition(fileId: string, x: number, y: number): void {
    // Update local state immediately for responsive UI
    this.positions.set(fileId, { x, y });

    // Clear any pending save for this file
    const pending = this.pendingSaves.get(fileId);
    if (pending) {
      clearTimeout(pending);
    }

    // Debounce the VFS write
    const timeout = setTimeout(() => {
      this.savePositionToVFS(fileId, x, y);
      this.pendingSaves.delete(fileId);
    }, this.POSITION_SAVE_DEBOUNCE_MS);

    this.pendingSaves.set(fileId, timeout);
  }

  /**
   * Internal method to persist position to VFS.
   */
  private async savePositionToVFS(
    fileId: string,
    x: number,
    y: number
  ): Promise<void> {
    const file = this.files.get(fileId);

    // Allow saving position even if file isn't loaded yet
    // This handles the case where file is uploaded but directory watcher hasn't fired
    const filePath = file?.path || `${DESKTOP_DIRECTORY}/${fileId}`;

    const positionPath = `${LAYOUT_DIRECTORY}/${fileId}.json`;
    const positionData: PositionData = {
      x,
      y,
      fileId,
      filePath,
      updatedAt: Date.now(),
    };

    try {
      const vfs = getVFSService();
      const exists = await vfs.exists(positionPath);

      await vfs.writeFile(
        positionPath,
        { content: positionData },
        !exists // create if doesn't exist
      );

      console.log('[DesktopService] Position saved:', fileId, { x, y });
    } catch (error) {
      console.error('[DesktopService] Failed to save position:', error);
      // Don't throw - position is already updated in local state
    }
  }

  /**
   * Get position for a file.
   */
  getPosition(fileId: string): { x: number; y: number } | null {
    return this.positions.get(fileId) || null;
  }

  /**
   * Get all files.
   */
  getFiles(): DesktopFile[] {
    return Array.from(this.files.values());
  }

  /**
   * Called when a new file is added to user directory.
   * Loads the file and creates a position with auto-layout if needed.
   */
  async onFileAdded(path: string): Promise<void> {
    const vfs = getVFSService();
    const fileId = this.pathToFileId(path);

    // Load the file immediately
    try {
      const doc = await vfs.readFile(path);
      const file = extractDesktopFile(path, doc);
      this.files.set(fileId, file);
      console.log('[DesktopService] Loaded new file:', fileId);
    } catch (error) {
      console.error('[DesktopService] Failed to load new file:', path, error);
      return;
    }

    // Create position if doesn't exist
    const existingPosition = this.positions.get(fileId);
    if (!existingPosition) {
      // Auto-layout: use index-based grid position
      const index = this.files.size;
      const { x, y } = getNextAutoLayoutPosition(index);

      console.log('[DesktopService] Auto-positioning new file:', fileId, {
        x,
        y,
      });
      this.setPosition(fileId, x, y);
    }

    // Notify listeners so Desktop creates the shape
    this.notifyListeners();
  }

  /**
   * Called when a file is deleted from user directory.
   * Removes the position file.
   */
  async onFileDeleted(fileId: string): Promise<void> {
    const positionPath = `${LAYOUT_DIRECTORY}/${fileId}.json`;
    const vfs = getVFSService();

    try {
      const exists = await vfs.exists(positionPath);
      if (exists) {
        await vfs.deleteFile(positionPath);
        console.log('[DesktopService] Position file deleted:', fileId);
      }

      // Clean up local state
      this.positions.delete(fileId);
      this.files.delete(fileId);
      this.notifyListeners();
    } catch (error) {
      console.error('[DesktopService] Failed to delete position file:', error);
    }
  }

  /**
   * Reload a file from VFS to refresh its metadata (e.g., thumbnail).
   * Call this after editing a file's content to update the desktop icon.
   */
  async reloadFile(path: string): Promise<void> {
    const vfs = getVFSService();
    const fileId = this.pathToFileId(path);

    // Only reload if file is already tracked
    if (!this.files.has(fileId)) {
      return;
    }

    try {
      const doc = await vfs.readFile(path);
      const file = extractDesktopFile(path, doc);
      this.files.set(fileId, file);
      this.notifyListeners();
      console.log('[DesktopService] Reloaded file:', fileId);
    } catch (error) {
      console.warn('[DesktopService] Failed to reload file:', path, error);
    }
  }

  /**
   * Cleanup and stop watching.
   */
  async destroy(): Promise<void> {
    const vfs = getVFSService();

    // Clear pending saves
    for (const timeout of this.pendingSaves.values()) {
      clearTimeout(timeout);
    }
    this.pendingSaves.clear();

    if (this.filesWatcherId) {
      await vfs.unwatchDirectory(this.filesWatcherId);
      this.filesWatcherId = null;
    }

    if (this.layoutWatcherId) {
      await vfs.unwatchDirectory(this.layoutWatcherId);
      this.layoutWatcherId = null;
    }

    // Clean up position file watchers
    for (const [fileId, watchId] of this.positionFileWatchers.entries()) {
      try {
        await vfs.unwatchFile(watchId);
      } catch (err) {
        console.warn(
          `[DesktopService] Error unwatching position file ${fileId}:`,
          err
        );
      }
    }
    this.positionFileWatchers.clear();

    this.files.clear();
    this.positions.clear();
    this.listeners.clear();
    this.initialized = false;

    console.log('[DesktopService] Destroyed');
  }

  // ============================================================================
  // Private methods
  // ============================================================================

  private async ensureDirectories(): Promise<void> {
    const vfs = getVFSService();

    // Ensure user files directory exists
    const userDirExists = await vfs.exists(DESKTOP_DIRECTORY);
    if (!userDirExists) {
      console.log(
        '[DesktopService] Creating user files directory:',
        DESKTOP_DIRECTORY
      );
      await vfs.writeFile(`${DESKTOP_DIRECTORY}/.keep`, { content: {} }, true);
    }

    // Ensure layout directory exists
    const layoutDirExists = await vfs.exists(LAYOUT_DIRECTORY);
    if (!layoutDirExists) {
      console.log(
        '[DesktopService] Creating layout directory:',
        LAYOUT_DIRECTORY
      );
      await vfs.writeFile(`${LAYOUT_DIRECTORY}/.keep`, { content: {} }, true);
    }
  }

  private async loadFiles(): Promise<void> {
    const vfs = getVFSService();

    try {
      const entries = await vfs.listDirectory(DESKTOP_DIRECTORY);

      if (!Array.isArray(entries)) {
        console.error('[DesktopService] Invalid directory listing:', entries);
        return;
      }

      const filePromises = entries
        // biome-ignore lint/suspicious/noExplicitAny: VFS entry type is dynamic
        .filter((entry: any) => entry.type === 'document')
        // biome-ignore lint/suspicious/noExplicitAny: VFS entry type is dynamic
        .map(async (entry: any) => {
          try {
            const path = `${DESKTOP_DIRECTORY}/${entry.name}`;
            const doc = await vfs.readFile(path);
            return extractDesktopFile(path, doc);
          } catch (error) {
            console.error(
              `[DesktopService] Failed to load file ${entry.name}:`,
              error
            );
            return null;
          }
        });

      const results = await Promise.allSettled(filePromises);

      this.files.clear();
      results
        .filter(
          (r): r is PromiseFulfilledResult<DesktopFile> =>
            r.status === 'fulfilled' && r.value !== null
        )
        .forEach(r => {
          const file = r.value;
          const fileId = this.pathToFileId(file.path);
          this.files.set(fileId, file);
        });

      console.log('[DesktopService] Loaded', this.files.size, 'files');
    } catch (error) {
      console.error('[DesktopService] Failed to load files:', error);
    }
  }

  private async loadPositions(): Promise<void> {
    const vfs = getVFSService();

    try {
      const entries = await vfs.listDirectory(LAYOUT_DIRECTORY);

      if (!Array.isArray(entries)) {
        console.warn('[DesktopService] Layout directory is empty or invalid');
        return;
      }

      const positionPromises = entries
        .filter(
          // biome-ignore lint/suspicious/noExplicitAny: VFS entry type is dynamic
          (entry: any) =>
            entry.type === 'document' && entry.name.endsWith('.json')
        )
        // biome-ignore lint/suspicious/noExplicitAny: VFS entry type is dynamic
        .map(async (entry: any) => {
          try {
            const path = `${LAYOUT_DIRECTORY}/${entry.name}`;
            const doc = await vfs.readFile(path);
            // biome-ignore lint/suspicious/noExplicitAny: Document content structure is dynamic
            const content = doc.content as any;

            // Validate content has required fields
            if (
              content &&
              typeof content.x === 'number' &&
              typeof content.y === 'number' &&
              content.fileId
            ) {
              return content as PositionData;
            }

            console.warn(
              `[DesktopService] Invalid position data in ${entry.name}`
            );
            return null;
          } catch (error) {
            console.error(
              `[DesktopService] Failed to load position ${entry.name}:`,
              error
            );
            return null;
          }
        });

      const results = await Promise.allSettled(positionPromises);

      this.positions.clear();
      results
        .filter(
          (r): r is PromiseFulfilledResult<PositionData> =>
            r.status === 'fulfilled' && r.value !== null
        )
        .forEach(r => {
          const data = r.value;
          this.positions.set(data.fileId, { x: data.x, y: data.y });
        });

      console.log('[DesktopService] Loaded', this.positions.size, 'positions');
    } catch (error) {
      console.error('[DesktopService] Failed to load positions:', error);
    }
  }

  private async startWatching(): Promise<void> {
    const vfs = getVFSService();

    // Watch user files directory (note: only fires for local changes, not remote)
    this.filesWatcherId = await vfs.watchDirectory(
      DESKTOP_DIRECTORY,
      changeData => {
        console.log(
          '[DesktopService] ⚡ Files directory changed (local):',
          changeData
        );
        this.handleFilesChange();
      }
    );
    console.log(
      '[DesktopService] ✅ Watching files directory:',
      DESKTOP_DIRECTORY,
      'watchId:',
      this.filesWatcherId
    );

    // Watch layout directory for add/remove
    this.layoutWatcherId = await vfs.watchDirectory(LAYOUT_DIRECTORY, () => {
      console.log('[DesktopService] ⚡ Layout directory changed');
      this.handleLayoutChange();
    });
    console.log(
      '[DesktopService] ✅ Watching layout directory:',
      LAYOUT_DIRECTORY,
      'watchId:',
      this.layoutWatcherId
    );

    // Watch individual position files for updates
    await this.setupPositionFileWatchers();

    console.log(
      '[DesktopService] 🎯 All watchers active - files:',
      this.filesWatcherId,
      'layout:',
      this.layoutWatcherId,
      'positions:',
      this.positionFileWatchers.size
    );
  }

  private async setupPositionFileWatchers(): Promise<void> {
    const vfs = getVFSService();

    // Clean up existing watchers
    for (const [fileId, watchId] of this.positionFileWatchers.entries()) {
      try {
        await vfs.unwatchFile(watchId);
      } catch (err) {
        console.warn(
          `[DesktopService] Error unwatching position file ${fileId}:`,
          err
        );
      }
    }
    this.positionFileWatchers.clear();

    // Set up watchers for each file's position
    for (const [fileId, _file] of this.files.entries()) {
      const positionPath = `${LAYOUT_DIRECTORY}/${fileId}.json`;

      try {
        const exists = await vfs.exists(positionPath);
        if (!exists) continue;

        const watchId = await vfs.watchFile(
          positionPath,
          async documentData => {
            console.log(
              '[DesktopService] Position file changed:',
              fileId,
              documentData
            );

            // Check if file was deleted (VFS might send null/empty on deletion)
            if (!documentData || !documentData.content) {
              console.log(
                '[DesktopService] Position file deleted remotely:',
                fileId
              );
              // File was deleted - clean up
              this.positions.delete(fileId);
              this.files.delete(fileId);
              this.positionFileWatchers.delete(fileId);
              this.notifyListeners();
              return;
            }

            // biome-ignore lint/suspicious/noExplicitAny: Document content structure is dynamic
            const content = documentData.content as any;
            if (
              content &&
              typeof content.x === 'number' &&
              typeof content.y === 'number'
            ) {
              // Update local state
              this.positions.set(fileId, { x: content.x, y: content.y });
              // Notify listeners
              this.notifyListeners();
            }
          }
        );

        this.positionFileWatchers.set(fileId, watchId);
        console.log(`[DesktopService] Watching position file: ${fileId}`);
      } catch (err) {
        console.error(
          `[DesktopService] Error setting up watcher for ${fileId}:`,
          err
        );
      }
    }

    console.log(
      '[DesktopService] Position file watchers set up:',
      this.positionFileWatchers.size
    );
  }

  private async handleFilesChange(): Promise<void> {
    const previousFiles = new Set(this.files.keys());
    await this.loadFiles();
    const currentFiles = new Set(this.files.keys());

    // Detect new files
    for (const fileId of currentFiles) {
      if (!previousFiles.has(fileId)) {
        const file = this.files.get(fileId);
        if (file) {
          console.log('[DesktopService] New file detected:', fileId);
          await this.onFileAdded(file.path);
        }
      }
    }

    // Detect deleted files
    for (const fileId of previousFiles) {
      if (!currentFiles.has(fileId)) {
        console.log('[DesktopService] File deleted detected:', fileId);
        await this.onFileDeleted(fileId);
      }
    }

    // Re-setup position file watchers if files changed
    if (previousFiles.size !== currentFiles.size) {
      await this.setupPositionFileWatchers();
    }

    this.notifyListeners();
  }

  private async handleLayoutChange(): Promise<void> {
    await this.loadPositions();
    this.notifyListeners();
  }

  private notifyListeners(): void {
    const state = this.getState();
    for (const listener of this.listeners) {
      try {
        listener(state);
      } catch (error) {
        console.error('[DesktopService] Listener error:', error);
      }
    }
  }

  private pathToFileId(path: string): string {
    // Extract filename and use as ID
    const fileName = path.split('/').pop() || path;
    // Keep extension for dotfiles (.keep, .gitignore), remove for others
    return fileName.startsWith('.')
      ? fileName
      : fileName.replace(/\.[^.]+$/, '');
  }
}

// Singleton instance
let desktopServiceInstance: DesktopService | null = null;

export function getDesktopService(): DesktopService {
  if (!desktopServiceInstance) {
    desktopServiceInstance = new DesktopService();
  }
  return desktopServiceInstance;
}

export function resetDesktopService(): void {
  if (desktopServiceInstance) {
    desktopServiceInstance.destroy();
    desktopServiceInstance = null;
  }
}

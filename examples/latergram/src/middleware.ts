import { StateCreator } from 'zustand';
import { getVFSService } from './services/vfs-service';

interface SyncOptions {
  path: string;
}

export const sync =
  <T extends object>(
    config: StateCreator<T>,
    options: SyncOptions
  ): StateCreator<T> =>
  (set, get, api) => {
    console.log(`Sync middleware initializing for path: ${options.path}`);
    const vfs = getVFSService();
    let watchId: string | null = null;
    let isInitialized = false;

    // Helper to serialize state for storage
    const serializeState = (state: T): string => {
      // Remove functions and non-serializable values
      const serializable = JSON.parse(JSON.stringify(state));
      return JSON.stringify(serializable);
    };

    // Helper to save state to file
    const saveToFile = async (state: T, create = false) => {
      if (!vfs.isInitialized()) return;

      try {
        const content = serializeState(state);
        // Try to update first, create if it doesn't exist
        try {
          await vfs.writeFile(options.path, content, create);
        } catch (error) {
          // If update fails, try creating the file
          await vfs.writeFile(options.path, content, true);
        }
      } catch (error) {
        console.warn(`Error saving state to ${options.path}:`, error);
      }
    };

    // Helper to load state from file
    const loadFromFile = async (): Promise<Partial<T> | null> => {
      if (!vfs.isInitialized()) return null;

      try {
        const content = await vfs.readFile(options.path);
        if (content) {
          return JSON.parse(content) as Partial<T>;
        }
      } catch (error) {
        // File might not exist yet, that's okay
        console.log(
          `No existing state file at ${options.path}, starting fresh`
        );
      }
      return null;
    };

    // Setup file watcher for external changes
    const setupWatcher = async () => {
      if (!vfs.isInitialized()) return;

      try {
        watchId = await vfs.watchFile(options.path, (content: string) => {
          try {
            const parsedState = JSON.parse(content) as Partial<T>;
            // Merge external changes into current state
            const currentState = get();
            const mergedState = { ...currentState, ...parsedState };
            set(mergedState as T, true); // Replace entire state
          } catch (error) {
            console.warn(
              `Error parsing external changes from ${options.path}:`,
              error
            );
          }
        });
      } catch (error) {
        console.warn(
          `Error setting up file watcher for ${options.path}:`,
          error
        );
      }
    };

    // Initialize state from file if it exists
    const initializeFromFile = async (state: T) => {
      console.log(`Checking if file exists: ${options.path}`);
      if (await vfs.exists(options.path)) {
        console.log(`File exists, loading saved state from ${options.path}`);
        const savedState = await loadFromFile();
        if (savedState) {
          // Use the already created state instead of calling config again
          const initialState = state;

          // Merge saved state with initial state, but preserve functions from initial state
          const mergedState = { ...initialState };

          // Only merge non-function properties from saved state
          Object.keys(savedState).forEach(key => {
            const savedValue = savedState[key as keyof typeof savedState];
            const initialValue = initialState[key as keyof typeof initialState];

            // Only merge if the initial value is not a function
            if (typeof initialValue !== 'function') {
              (mergedState as Record<string, unknown>)[key] = savedValue;
            }
          });

          set(mergedState as T, true);
          console.log(`Loaded and merged state from ${options.path}`);
        }
      } else {
        console.log(`File doesn't exist, creating new file: ${options.path}`);
        await saveToFile(state, true);
        console.log(`Created new file: ${options.path}`);
      }

      // Setup file watcher after initial load
      await setupWatcher();
      isInitialized = true;
      console.log(`Sync middleware fully initialized for ${options.path}`);
    };

    // Create the initial state
    const wrappedSet = (
      partial: T | Partial<T> | ((state: T) => T | Partial<T>),
      replace?: boolean
    ) => {
      // console.log("Set called with:", { partial, replace });

      // Apply changes to Zustand state first
      if (replace === true) {
        set(partial as T, replace);
      } else {
        set(partial);
      }

      // Auto-save to file after state change
      if (isInitialized) {
        const currentState = get();
        saveToFile(currentState);
      }
    };

    // console.log("Creating state with wrapped set function");
    const state = config(wrappedSet, get, api);

    // Initialize from file when VFS is ready
    const waitForVFS = () => {
      if (vfs.isInitialized()) {
        console.log(`VFS ready, initializing from file for ${options.path}`);
        initializeFromFile(state);
      } else {
        // console.log(`VFS not ready for ${options.path}, waiting...`);
        // Poll until VFS is ready
        setTimeout(waitForVFS, 100);
      }
    };
    waitForVFS();

    // Note: Cleanup would be needed if Zustand had a built-in cleanup mechanism
    // For now, the watcher will remain active for the lifetime of the store

    // Add cleanup function to state for manual cleanup if needed
    (state as T & { __cleanup?: () => void }).__cleanup = () => {
      if (watchId) {
        vfs.unwatchFile(watchId).catch(console.warn);
        watchId = null;
      }
    };

    return state;
  };

import type { DocumentData } from '@tonk/core';
import type { StateCreator } from 'zustand';
import { getVFSService } from '@/vfs-client';

// biome-ignore lint/suspicious/noExplicitAny: Middleware types are complex
interface SyncOptions<T = any> {
  path: string;
  partialize?: (state: T) => Partial<T>;
}

export const sync =
  <T extends object>(
    // biome-ignore lint/suspicious/noExplicitAny: Middleware types are complex
    config: StateCreator<T, any, any>,
    options?: SyncOptions
    // biome-ignore lint/suspicious/noExplicitAny: Middleware types are complex
  ): StateCreator<T, any, any> =>
  (set, get, api) => {
    // If no options provided, just return the config without sync
    if (!options || !options.path) {
      console.warn('Sync middleware called without path option, skipping sync functionality');
      return config(set, get, api);
    }

    console.log(`Sync middleware initializing for path: ${options.path}`);
    const vfs = getVFSService();
    let watchId: string | null = null;
    let isInitialized = false;
    let initializationPromise: Promise<void> | null = null;
    let connectionStateUnsubscribe: (() => void) | null = null;
    let saveTimeout: ReturnType<typeof setTimeout> | null = null;
    const SAVE_DEBOUNCE_MS = 2000; // Debounce saves by 2 seconds

    // Helper to serialize state for storage
    // biome-ignore lint/suspicious/noExplicitAny: State type is dynamic
    const serializeState = (state: T): any => {
      // Use partialize if provided, otherwise use full state
      const stateToSerialize = options.partialize ? options.partialize(state) : state;
      // Remove functions and non-serializable values
      const serializable = JSON.parse(JSON.stringify(stateToSerialize));
      return serializable;
    };

    // Helper to save state to file (debounced)
    const saveToFile = (state: T, create = false) => {
      if (!vfs.isInitialized()) return;

      // Clear any pending save
      if (saveTimeout) {
        clearTimeout(saveTimeout);
      }

      // Debounce the save
      saveTimeout = setTimeout(
        async () => {
          try {
            const content = serializeState(state);

            if (create) {
              // First save: create full document
              await vfs.writeFile(options.path, { content }, true);
              console.log(`[sync] Created new file: ${options.path}`);
              return;
            }

            await vfs.updateFile(options.path, content);
          } catch (error) {
            console.warn(`Error saving state to ${options.path}:`, error);
          }
        },
        create ? 0 : SAVE_DEBOUNCE_MS
      ); // Immediate on create, debounced on update
    };

    // Helper to load state from file
    const loadFromFile = async (): Promise<Partial<T> | null> => {
      if (!vfs.isInitialized()) return null;

      try {
        const content = await vfs.readFile(options.path);
        if (content) {
          return content.content as Partial<T>;
        }
      } catch {
        // File might not exist yet, that's okay
        console.log(`No existing state file at ${options.path}, starting fresh`);
      }
      return null;
    };

    // Setup file watcher for external changes
    const setupWatcher = async () => {
      if (!vfs.isInitialized()) return;

      if (watchId) {
        try {
          await vfs.unwatchFile(watchId);
          watchId = null;
        } catch (error) {
          console.warn(`Error unwatching previous watcher for ${options.path}:`, error);
        }
      }

      try {
        watchId = await vfs.watchFile(options.path, (content: DocumentData) => {
          try {
            const parsedState = content.content as Partial<T>;
            // Merge external changes into current state
            const currentState = get();
            const mergedState = { ...currentState, ...parsedState };
            // biome-ignore lint/suspicious/noExplicitAny: State type is dynamic
            set(mergedState as T, true as any); // Replace entire state
          } catch (error) {
            console.warn(`Error parsing external changes from ${options.path}:`, error);
          }
        });
      } catch (error) {
        console.warn(`Error setting up file watcher for ${options.path}:`, error);
      }
    };

    // Initialize state from file if it exists
    const initializeFromFile = (state: T): Promise<void> => {
      // If already initialized or currently initializing, return existing promise or resolve
      if (isInitialized) {
        console.log(`[sync] Already initialized for ${options.path}, skipping`);
        return Promise.resolve();
      }
      if (initializationPromise) {
        return initializationPromise;
      }

      // Create and store the initialization promise
      initializationPromise = (async () => {
        try {
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
              Object.keys(savedState).forEach((key) => {
                const savedValue = savedState[key as keyof typeof savedState];
                const initialValue = initialState[key as keyof typeof initialState];

                // Only merge if the initial value is not a function
                if (typeof initialValue !== 'function') {
                  (mergedState as Record<string, unknown>)[key] = savedValue;
                }
              });

              // biome-ignore lint/suspicious/noExplicitAny: State type is dynamic
              set(mergedState as T, true as any);
              console.log(`Loaded and merged state from ${options.path}`);
            }
          } else {
            console.log(`File doesn't exist, creating new file: ${options.path}`);
            saveToFile(state, true);
            console.log(`Created new file: ${options.path}`);
          }

          // Setup file watcher after initial load
          await setupWatcher();
          isInitialized = true;
          console.log(`Sync middleware fully initialized for ${options.path}`);
        } finally {
          initializationPromise = null;
        }
      })();

      return initializationPromise;
    };

    // Create the initial state
    const wrappedSet = (
      partial: T | Partial<T> | ((state: T) => T | Partial<T>),
      replace?: boolean
    ) => {
      // Apply changes to Zustand state first
      if (replace === true) {
        // TODO: replace full overwrite with more intelligent deep replacement
        // biome-ignore lint/suspicious/noExplicitAny: State type is dynamic
        set(partial as T, replace as any);
        return; // Skip saveToFile to prevent watcher feedback loop
      } else {
        set(partial);
      }

      // Auto-save to file after state change
      if (isInitialized) {
        const currentState = get();
        saveToFile(currentState);
      }
    };

    const state = config(wrappedSet, get, api);

    // Listen for connection state changes
    connectionStateUnsubscribe = vfs.onConnectionStateChange(async (connectionState) => {
      if (connectionState === 'reconnecting') {
        // TonkCore is being switched - reset so we reload from new VFS
        console.log(`[sync] Connection reconnecting, resetting state for ${options.path}`);
        isInitialized = false;
        initializationPromise = null;
        if (watchId) {
          // Old watcher is dead anyway, just clear the reference
          watchId = null;
        }
        if (saveTimeout) {
          clearTimeout(saveTimeout);
          saveTimeout = null;
        }
      } else if (connectionState === 'connected' && !isInitialized) {
        await initializeFromFile(state);
      } else if (connectionState === 'connected' && isInitialized) {
        await setupWatcher();
      }
    });

    // Initialize from file when VFS is ready
    const waitForVFS = () => {
      console.log('waiting for vfs', vfs);
      if (vfs.isInitialized()) {
        console.log('init from file start');
        initializeFromFile(state);
        return;
      }
      setTimeout(waitForVFS, 100);
    };
    waitForVFS();

    // Add cleanup function to state for manual cleanup if needed
    (state as T & { __cleanup?: () => void }).__cleanup = () => {
      if (watchId) {
        vfs.unwatchFile(watchId).catch(console.warn);
        watchId = null;
      }
      if (connectionStateUnsubscribe) {
        connectionStateUnsubscribe();
        connectionStateUnsubscribe = null;
      }
      if (saveTimeout) {
        clearTimeout(saveTimeout);
        saveTimeout = null;
      }
    };

    // Add reset function for when TonkCore changes (e.g., switching between tonks)
    // This resets initialization state so the store reloads from the new VFS
    (state as T & { __reset?: () => void }).__reset = () => {
      console.log(`[sync] Resetting middleware for ${options.path}`);
      isInitialized = false;
      initializationPromise = null;
      if (watchId) {
        vfs.unwatchFile(watchId).catch(console.warn);
        watchId = null;
      }
      if (saveTimeout) {
        clearTimeout(saveTimeout);
        saveTimeout = null;
      }
    };

    return state;
  };

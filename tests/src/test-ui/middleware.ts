import type { DocumentData } from '@tonk/core';
import type { StateCreator } from 'zustand';
import { getVFSService } from './vfs-service';

interface SyncOptions {
  path: string;
}

// const isLocalhost =
//   window.location.hostname === 'localhost' ||
//   window.location.hostname === '127.0.0.1';
// const relayUrl = isLocalhost
//   ? 'http://localhost:8081'
//   : 'https://relay.tonk.xyz';
const relayUrl = 'https://relay.tonk.xyz';

const manifestUrl = `${relayUrl}/.manifest.tonk`;
const wsUrl = relayUrl.replace(/^http/, 'ws');

export const sync =
  <T extends object>(
    config: StateCreator<T>,
    options?: SyncOptions
  ): StateCreator<T> =>
  (set, get, api) => {
    // If no options provided, just return the config without sync
    if (!options || !options.path) {
      console.warn(
        'Sync middleware called without path option, skipping sync functionality'
      );
      return config(set, get, api);
    }

    console.log(`Sync middleware initializing for path: ${options.path}`);
    const vfs = getVFSService();
    vfs.initialize(manifestUrl, wsUrl);
    let watchId: string | null = null;
    let isInitialized = false;
    let connectionStateUnsubscribe: (() => void) | null = null;

    // Helper to serialize state for storage
    const serializeState = (state: T): any => {
      // Remove functions and non-serializable values
      const serializable = JSON.parse(JSON.stringify(state));
      return serializable;
    };

    // Helper to save state to file
    const saveToFile = async (state: T, create = false) => {
      if (!vfs.isInitialized()) return;

      try {
        const content = serializeState(state);
        // Try to update first, create if it doesn't exist
        try {
          await vfs.writeFile(options.path, { content }, create);
        } catch {
          // If update fails, try creating the file
          await vfs.writeFile(options.path, { content }, true);
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
          return content.content as Partial<T>;
        }
      } catch {
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

      if (watchId) {
        try {
          await vfs.unwatchFile(watchId);
          watchId = null;
        } catch (error) {
          console.warn(
            `Error unwatching previous watcher for ${options.path}:`,
            error
          );
        }
      }

      try {
        watchId = await vfs.watchFile(options.path, (content: DocumentData) => {
          try {
            const parsedState = content.content as Partial<T>;
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
      // Guard against multiple initializations
      if (isInitialized) {
        console.log(`Already initialized for ${options.path}, skipping`);
        return;
      }

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

    const state = config(wrappedSet, get, api);

    // Listen for connection state changes
    connectionStateUnsubscribe = vfs.onConnectionStateChange(
      async connectionState => {
        if (connectionState === 'connected' && !isInitialized) {
          await initializeFromFile(state);
        } else if (connectionState === 'connected' && isInitialized) {
          await setupWatcher();
        }
      }
    );

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
    };

    return state;
  };

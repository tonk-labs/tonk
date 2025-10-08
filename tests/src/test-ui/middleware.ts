import type { DocumentData } from '@tonk/core';
import type { StateCreator } from 'zustand';
import { getVFSService } from './vfs-service';

interface SyncOptions {
  path: string;
}

export const sync =
  <T extends object>(
    config: StateCreator<T>,
    options?: SyncOptions
  ): StateCreator<T> =>
  (set, get, api) => {
    if (!options || !options.path) {
      console.warn(
        'Sync middleware called without path option, skipping sync functionality'
      );
      return config(set, get, api);
    }

    console.log(`Sync middleware initializing for path: ${options.path}`);
    const vfs = getVFSService();
    let watchId: string | null = null;
    let isInitialized = false;

    const serializeState = (state: T): any => {
      const serializable = JSON.parse(JSON.stringify(state));
      return serializable;
    };

    const saveToFile = async (state: T, create = false) => {
      if (!vfs.isInitialized()) return;

      try {
        const content = serializeState(state);
        try {
          await vfs.writeFile(options.path, { content }, create);
        } catch (error) {
          await vfs.writeFile(options.path, { content }, true);
        }
      } catch (error) {
        console.warn(`Error saving state to ${options.path}:`, error);
      }
    };

    const loadFromFile = async (): Promise<Partial<T> | null> => {
      if (!vfs.isInitialized()) return null;

      try {
        const content = await vfs.readFile(options.path);
        if (content) {
          return content.content as Partial<T>;
        }
      } catch (error) {
        console.log(
          `No existing state file at ${options.path}, starting fresh`
        );
      }
      return null;
    };

    const setupWatcher = async () => {
      if (!vfs.isInitialized()) return;

      try {
        watchId = await vfs.watchFile(options.path, (content: DocumentData) => {
          try {
            const parsedState = content.content as Partial<T>;
            const currentState = get();
            const mergedState = { ...currentState, ...parsedState };
            set(mergedState as T, true);
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

    const initializeFromFile = async (state: T) => {
      console.log(`Checking if file exists: ${options.path}`);
      if (await vfs.exists(options.path)) {
        console.log(`File exists, loading saved state from ${options.path}`);
        const savedState = await loadFromFile();
        if (savedState) {
          const initialState = state;
          const mergedState = { ...initialState };

          Object.keys(savedState).forEach(key => {
            const savedValue = savedState[key as keyof typeof savedState];
            const initialValue = initialState[key as keyof typeof initialState];

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

      await setupWatcher();
      isInitialized = true;
      console.log(`Sync middleware fully initialized for ${options.path}`);
    };

    const wrappedSet = (
      partial: T | Partial<T> | ((state: T) => T | Partial<T>),
      replace?: boolean
    ) => {
      if (replace === true) {
        set(partial as T, replace);
      } else {
        set(partial);
      }

      if (isInitialized) {
        const currentState = get();
        saveToFile(currentState);
      }
    };

    const state = config(wrappedSet, get, api);

    const waitForVFS = () => {
      if (vfs.isInitialized()) {
        console.log(`VFS ready, initializing from file for ${options.path}`);
        initializeFromFile(state);
      } else {
        setTimeout(waitForVFS, 100);
      }
    };
    waitForVFS();

    (state as T & { __cleanup?: () => void }).__cleanup = () => {
      if (watchId) {
        vfs.unwatchFile(watchId).catch(console.warn);
        watchId = null;
      }
    };

    return state;
  };

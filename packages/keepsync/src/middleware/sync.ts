/**
 * Zustand state creator type
 * Represents the function that creates the initial state and actions for a Zustand store
 */
import {StateCreator} from 'zustand';

/**
 * Import Repo and related types from automerge-repo
 */
import {Repo, DocHandle, DocumentId} from '@automerge/automerge-repo';

/**
 * Utility functions for state management:
 * - patchStore: Updates Zustand store with changes from Automerge
 * - removeNonSerializable: Prepares state for serialization by removing functions and non-serializable values
 */
import {patchStore, removeNonSerializable} from './patching.js';

import {
  findDocument,
  createDocument,
  DocNode,
  RefNode,
  DirNode,
  traverseDocTree,
  removeDocument,
} from '../documents/addressing.js';

/**
 * Logger utility for consistent logging throughout the application
 */
import {logger} from '../utils/logger.js';

/**
 * Function to get the Repo instance
 */
import {getRepo, getRootId} from '../core/syncConfig.js';

/**
 * Configuration options for the sync middleware
 */
export interface SyncOptions {
  /**
   * Unique document ID for syncing this store
   * This ID is used to identify the document in the sync engine and across peers
   */
  docId: DocumentId;

  /**
   * Maximum time to wait for Repo initialization (in milliseconds)
   * After this time, the initialization will time out and call onInitError
   * @default 30000 (30 seconds)
   */
  initTimeout?: number;

  /**
   * Callback function that is called when sync initialization fails
   * This can be due to timeout or other errors during initialization
   * @param error The error that occurred during initialization
   */
  onInitError?: (error: Error) => void;
}

/**
 * Middleware for syncing Zustand stores with Automerge
 *
 * This middleware creates a bidirectional sync between a Zustand store and an Automerge document:
 * 1. Changes to the Zustand store are automatically synced to Automerge
 * 2. Changes from Automerge (from other peers) are automatically applied to the Zustand store
 *
 * The sync is established when the Repo becomes available, which may not be immediate.
 * The middleware handles the initialization timing and retries automatically.
 *
 * @example
 * const useStore = create(
 *   sync(
 *     (set) => ({
 *       count: 0,
 *       increment: () => set(state => ({ count: state.count + 1 })),
 *     }),
 *     { docId: 'counter' }
 *   )
 * );
 *
 * @param config - The Zustand state creator function that defines the store's state and actions
 * @param options - Configuration options for the sync middleware
 * @returns A wrapped state creator function that includes sync functionality
 */
export const sync =
  <T extends object>(
    config: StateCreator<T>,
    options: SyncOptions,
  ): StateCreator<T> =>
  (set, get, api) => {
    // Reference to the DocHandle
    let docHandle: DocHandle<T> | null = null;

    // Flag to track if sync has been initialized
    // Prevents duplicate initialization and unnecessary updates before initialization
    let isSyncInit = false;

    // Create the initial state by calling the original config function with our wrapped set function
    const state = config(
      // Wrap the original set function to sync changes to Automerge whenever state changes
      (partial, replace) => {
        // Step 1: Apply changes to Zustand state first (local state update)
        set(partial as any, replace as any);

        // Step 2: Check if we can sync to Automerge
        // Skip syncing if the docHandle isn't available or sync hasn't been initialized yet
        if (!docHandle || !isSyncInit) return;

        // Step 3: Get the current complete state to update Automerge
        const currentState = get();
        try {
          // Remove functions and non-serializable values before syncing
          // This is important because Automerge can only store serializable data
          const serializableState = removeNonSerializable(currentState);

          // Step 4: Update the Automerge document through the docHandle
          docHandle.change((doc: any) => {
            // Merge the serializable state into the Automerge document
            Object.assign(doc, serializableState);
          });
        } catch (error) {
          // Handle errors that might occur during serialization or update
          logger.warn(`Error updating document ${options.docId}:`, error);
        }
      },
      // Pass through the original get and api functions
      get,
      api,
    );

    /**
     * Handles changes coming from Automerge to update the Zustand store
     *
     * This function is called when:
     * 1. The document is first loaded
     * 2. Changes are received from other peers via the sync engine
     *
     * It updates the Zustand store without triggering the sync-back mechanism.
     *
     * @param updatedDoc - The updated Automerge document
     */
    const handleDocChange = (updatedDoc: any) => {
      // Safety check - skip if we received a null/undefined document
      if (!updatedDoc) return;

      try {
        // Log the incoming changes for debugging
        logger.debugWithContext(
          'sync-middleware',
          'Received doc change, updating Zustand store:',
          updatedDoc,
        );

        // Update the Zustand store with the new data
        // patchStore only updates state synced with Automerge and avoids
        // our custom set function, which would cause an infinite loop
        patchStore(api, updatedDoc);
      } catch (error) {
        // Log any errors that occur during the update process
        logger.error(
          `Error handling document change for ${options.docId}:`,
          error,
        );
      }
    };

    /**
     * Initializes the synchronization between Zustand and the Automerge document
     *
     * This function:
     * 1. Gets or creates the document handle from the repo
     * 2. Sets up callbacks to handle changes from other peers
     * 3. Marks the sync as initialized when complete
     */
    async function initializeSync() {
      // Skip if already initialized to prevent duplicate initialization
      if (isSyncInit) return;

      // Get the repo instance
      const repo = getRepo();
      const root = getRootId()!;

      // If the repo is not available, log a warning and exit
      if (!repo) {
        logger.warn(
          `Cannot initialize sync for ${options.docId}: repo not available`,
        );
        return;
      }

      try {
        // Get the document handle from the repo or create a new one if it doesn't exist
        let docNode = await findDocument<T>(repo, root, options.docId);
        if (!docNode) {
          docHandle = repo.create<T>();
          createDocument(repo, root, options.docId, docHandle);
        } else {
          docHandle = docNode;
          await docNode.doc();
        }

        // Set up the change callback to handle document updates
        docHandle?.on('change', ({doc}) => {
          if (doc) {
            handleDocChange(doc);
          }
        });

        // Get the current document
        const currentDoc = docHandle?.docSync();

        if (currentDoc) {
          // CASE 1: Document already exists - update the Zustand store with its contents
          handleDocChange(currentDoc);
        } else {
          // CASE 2: Document doesn't exist yet - initialize it with current Zustand state
          const initialState = get();
          const serializableState = removeNonSerializable(initialState);

          // Create the initial document with the current state
          docHandle?.change((doc: any) => {
            Object.assign(doc, serializableState);
          });
        }

        // Mark initialization as complete
        isSyncInit = true;
        logger.debug(`Sync initialized for document ${options.docId}`);
      } catch (err) {
        // Handle any errors during initialization
        logger.error(`Failed to initialize document ${options.docId}:`, err);

        // Call the error callback if provided
        if (options.onInitError) {
          // Ensure we always pass an Error object
          options.onInitError(
            err instanceof Error ? err : new Error(String(err)),
          );
        }
      }
    }

    // Configuration for the initialization process
    // Maximum time to wait for the repo to become available
    const MAX_INIT_TIME = options.initTimeout ?? 30000; // Default 30 seconds timeout

    // Timer reference for cleanup and cancellation
    let initTimer: ReturnType<typeof setTimeout> | null = null;

    // Track when we started trying to initialize
    let initStartTime = Date.now();

    /**
     * Global Registry Setup
     *
     * This creates a global registry to track callbacks waiting for the repo.
     * It's more efficient than having each store instance poll independently.
     */
    if (typeof window !== 'undefined' && !window.__REPO_REGISTRY__) {
      // Create the registry if it doesn't exist yet
      window.__REPO_REGISTRY__ = {
        // Array of callbacks to call when the repo becomes available
        callbacks: [],

        // Function to notify all waiting callbacks
        notifyCallbacks: function () {
          const repo = getRepo();
          if (repo) {
            // Call each callback with the repo instance
            this.callbacks.forEach(cb => cb(repo));
            // Clear the callbacks after notifying them
            this.callbacks = [];
          }
        },
      };

      // Set up a global interval to check for repo availability
      const checkInterval = setInterval(() => {
        const repo = getRepo();
        // When the repo becomes available, notify all waiting callbacks
        if (repo && window.__REPO_REGISTRY__) {
          window.__REPO_REGISTRY__.notifyCallbacks();
          // Stop checking once the repo is available
          clearInterval(checkInterval);
        }
      }, 100); // Check every 100ms
    }

    /**
     * Attempts to initialize sync with a timeout mechanism
     *
     * This function:
     * 1. Checks if we've exceeded the maximum wait time
     * 2. Tries to initialize sync if the repo is available
     * 3. Sets up callbacks or timers to retry if the repo isn't available yet
     */
    const initSyncWithTimeout = async () => {
      // STEP 1: Check if we've exceeded the timeout period
      if (Date.now() - initStartTime > MAX_INIT_TIME) {
        // Clean up any pending timers
        if (initTimer) {
          clearTimeout(initTimer);
          initTimer = null;
        }

        // Create and log a timeout error
        const timeoutError = new Error(
          `Sync initialization timed out after ${MAX_INIT_TIME}ms for document ${options.docId}`,
        );
        logger.error(timeoutError.message);

        // Notify the caller about the timeout via the error callback
        if (options.onInitError) {
          options.onInitError(timeoutError);
        }
        return; // Exit the function, giving up on initialization
      }

      // STEP 2: Check if the repo is available now
      const repo = getRepo();

      if (repo && !isSyncInit) {
        // CASE 1: Repo is available and we haven't initialized yet
        logger.debug(`Repo available, initializing store for ${options.docId}`);

        // Initialize the sync
        await initializeSync();

        // Clean up any pending timers
        if (initTimer) {
          clearTimeout(initTimer);
          initTimer = null;
        }
      } else if (!repo) {
        // CASE 2: Repo is not available yet

        // In browser environments, use the registry mechanism
        if (typeof window !== 'undefined' && window.__REPO_REGISTRY__) {
          // Register a callback to be called when the repo becomes available
          window.__REPO_REGISTRY__.callbacks.push(async () => {
            if (!isSyncInit) {
              logger.debug(
                `Repo became available, initializing store for ${options.docId}`,
              );
              await initializeSync();
            }
          });

          // Set a backup timer in case the registry mechanism fails
          // This is less frequent (1 second) since it's just a backup
          initTimer = setTimeout(initSyncWithTimeout, 1000);
        } else {
          // In non-browser environments (Node.js, React Native, etc.)
          // Fall back to more frequent polling since we don't have the registry
          initTimer = setTimeout(initSyncWithTimeout, 100);
        }
      }
      // If repo is available but isSyncInit is true, we've already initialized
      // so we don't need to do anything
    };

    // Start the initialization process immediately
    initSyncWithTimeout();

    // Return the state to Zustand
    // The original state is enhanced with sync capabilities through the wrapped set function
    return {
      ...state,
      // We could add cleanup methods or sync status indicators here if needed
    };
  };

const addressingParams = () => {
  const repo = getRepo();
  const root = getRootId();

  if (!repo || !root) {
    throw new Error('SyncEngine is not properly initialized');
  }

  return {
    repo,
    root,
  };
};

export const ls = async (path: string): Promise<DocNode | undefined> => {
  const {repo, root} = addressingParams();

  const result = await traverseDocTree(repo, root, path);
  if (!result) {
    return undefined;
  }
  if (!result.targetRef) {
    if (result.node) {
      return result.node as DocNode;
    }
  }

  if (result.targetRef?.type === 'doc') {
    throw new Error('You cannot ls a document');
  }

  const dir = repo.find<DirNode>(result.targetRef!.pointer);

  return await dir.doc();
};

export const rm = async (path: string): Promise<boolean> => {
  const {repo, root} = addressingParams();
  return await removeDocument(repo, root, path);
};

export const mkDir = async (path: string): Promise<DirNode | undefined> => {
  const {repo, root} = addressingParams();
  const result = await traverseDocTree(repo, root, path, true);
  if (!result) {
    return undefined;
  }
  return result!.node;
};

/**
 * Reads a document from keepsync
 *
 * This function retrieves a document at the specified path from the repository.
 * It returns the document content if found, or undefined if the document doesn't exist.
 *
 * @param path - The path identifying the document to read
 * @returns Promise resolving to the document content or undefined if not found
 * @throws Error if the SyncEngine is not properly initialized
 */
export const readDoc = async <T>(path: string): Promise<T | undefined> => {
  const {repo, root} = addressingParams();

  const docNode = await findDocument<T>(repo, root, path);
  if (!docNode) {
    return undefined;
  }

  return await docNode.doc();
};

/**
 * Writes content to a document to keepsync
 *
 * This function creates or updates a document at the specified path.
 * If the document doesn't exist, it creates a new one.
 * If the document already exists, it updates it with the provided content.
 *
 * @param path - The path identifying the document to write
 * @param content - The content to write to the document
 * @throws Error if the SyncEngine is not properly initialized
 */
export const writeDoc = async <T>(path: string, content: T) => {
  const {repo, root} = addressingParams();

  let docNode = await findDocument<T>(repo, root, path);
  let newHandle;
  if (!docNode) {
    newHandle = repo.create<T>();
    await newHandle.doc();
    await createDocument(repo, root, path, newHandle);
  } else {
    newHandle = docNode;
    await newHandle.doc();
  }
  newHandle.change((doc: any) => {
    Object.assign(doc, content);
  });
  newHandle.docSync();
};

// Add necessary TypeScript interfaces for global registry
declare global {
  interface Window {
    __REPO_REGISTRY__?: {
      callbacks: ((repo: Repo) => void)[];
      notifyCallbacks: () => void;
    };
  }
}

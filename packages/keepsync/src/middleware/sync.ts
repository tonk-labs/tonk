/**
 * Automerge - CRDT library for managing distributed state
 * Used for handling the underlying document synchronization
 */
import * as Automerge from '@automerge/automerge';

/**
 * Zustand state creator type
 * Represents the function that creates the initial state and actions for a Zustand store
 */
import {StateCreator} from 'zustand';

/**
 * Utility functions for state management:
 * - patchStore: Updates Zustand store with changes from Automerge
 * - removeNonSerializable: Prepares state for serialization by removing functions and non-serializable values
 */
import {patchStore, removeNonSerializable} from './patching.js';

/**
 * Logger utility for consistent logging throughout the application
 */
import {logger} from '../utils/logger.js';

/**
 * Function to get the singleton instance of the sync engine
 * The sync engine might not be immediately available on initialization
 */
import {getSyncInstance} from '../engine/index.js';
import {AutomergeUrl, parseAutomergeUrl} from '@automerge/automerge-repo';
import bs58check from 'bs58check';
import {stringToUuidV4} from '../utils/uuid.js';
import * as Uuid from 'uuid';

/**
 * Configuration options for the sync middleware
 */
export interface SyncOptions {
  /**
   * Unique document ID for syncing this store
   * This ID is used to identify the document in the sync engine and across peers
   */
  docId: string;

  /**
   * Maximum time to wait for sync engine initialization (in milliseconds)
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
 * The sync is established when the sync engine becomes available, which may not be immediate.
 * The middleware handles the initialization timing and retries automatically.
 *
 * Document IDs can be dynamically managed through the docIdManager module:
 * - Document IDs can be prefixed globally (e.g., for multi-user isolation)
 * - Logical document IDs can be mapped to different actual IDs at runtime
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
    // The current Automerge document instance
    // This is null until initialization is complete
    let currentDoc: Automerge.Doc<T> | null = null;

    // Flag to track if sync has been initialized
    // Prevents duplicate initialization and unnecessary updates before initialization
    let isSyncInit = false;

    const resolvedUrl: AutomergeUrl = `automerge:${bs58check.encode(
      Uuid.parse(stringToUuidV4(options.docId)),
    )}` as AutomergeUrl;

    const resolvedClientId = parseAutomergeUrl(resolvedUrl).documentId;

    // Create the initial state by calling the original config function with our wrapped set function
    const state = config(
      // Wrap the original set function to sync changes to Automerge whenever state changes
      (partial, replace) => {
        // Step 1: Apply changes to Zustand state first (local state update)
        // Type assertion is needed because Zustand's types don't match perfectly with our generic T
        set(partial as any, replace as any);

        // Step 2: Check if we can sync to Automerge
        const syncEngine = getSyncInstance();
        // Skip syncing if the engine isn't available or sync hasn't been initialized yet
        if (!syncEngine || !isSyncInit) return;

        // Step 3: Get the current complete state to update Automerge
        const currentState = get();
        try {
          // Remove functions and non-serializable values before syncing
          // This is important because Automerge can only store serializable data
          const serializableState = removeNonSerializable(currentState);

          // Step 4: Update the Automerge document with the new state
          syncEngine
            .updateDocument(resolvedClientId, (doc: any) => {
              // Merge the serializable state into the Automerge document
              // This creates a new change in the Automerge document history
              Object.assign(doc, serializableState);
            })
            .catch(err => {
              // Log any errors that occur during the update process
              logger.warn(`Error updating document ${resolvedClientId}:`, err);
            });
        } catch (error) {
          // Handle errors that might occur during serialization
          logger.warn(
            `Error preparing update for document ${resolvedClientId}:`,
            error,
          );
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
     * It converts the Automerge document to plain JS and updates the Zustand store
     * without triggering the sync-back mechanism (to avoid loops).
     *
     * @param newDoc - The updated Automerge document
     */
    const handleDocChange = (newDoc: Automerge.Doc<T>) => {
      // Safety check - skip if we received a null/undefined document
      if (!newDoc) return;

      try {
        // Update our reference to the current document
        currentDoc = newDoc;
        logger.debug('Heads: ' + Automerge.getHeads(currentDoc));

        // Convert the Automerge document to a plain JavaScript object
        // This is necessary because Zustand works with plain objects, not Automerge docs
        const jsData = Automerge.toJS(newDoc);

        // Log the incoming changes for debugging
        logger.debugWithContext(
          'sync-middleware',
          'Received doc change, updating Zustand store:',
          jsData,
        );

        // Update the Zustand store with the new data
        // patchStore only updates state synced with Automerge and avoids
        // our custom set function, which would cause an infinite loop
        patchStore(api, jsData);
      } catch (error) {
        // Log any errors that occur during the update process
        logger.error(
          `Error handling document change for ${resolvedClientId}:`,
          error,
        );
      }
    };

    /**
     * Initializes the synchronization between Zustand and Automerge
     *
     * This function:
     * 1. Retrieves or creates the Automerge document
     * 2. Sets up callbacks to handle changes from other peers
     * 3. Marks the sync as initialized when complete
     *
     * The function is called when the sync engine becomes available.
     */
    function initializeSync() {
      // Skip if already initialized to prevent duplicate initialization
      if (isSyncInit) return;

      // Get the sync engine instance
      const syncEngine = getSyncInstance();

      // If the sync engine is not available, log a warning and exit
      // The initialization will be retried by the timeout mechanism
      if (!syncEngine) {
        logger.warn(
          `Cannot initialize sync for ${resolvedClientId}: sync engine not available`,
        );
        return;
      }

      // Start the initialization process
      syncEngine
        .getDocument(resolvedClientId)
        .then(existingDoc => {
          if (existingDoc) {
            // CASE 1: Document already exists in the sync engine
            // Store the document reference and update the Zustand store with its contents
            currentDoc = existingDoc;
            handleDocChange(existingDoc);
          } else {
            // CASE 2: Document doesn't exist yet, create a new one
            // Get the current state from Zustand to use as initial document state
            const initialState = get();

            // Create a new Automerge document with the initial state
            const newDoc = Automerge.change(Automerge.init<T>(), (doc: any) => {
              // Copy the serializable parts of the state to the new document
              Object.assign(doc, removeNonSerializable(initialState));
            });

            // Store the document reference
            currentDoc = newDoc;

            // Create the document in the sync engine
            // This returns a promise that resolves when the document is created
            return syncEngine!.createDocument(
              resolvedClientId,
              Automerge.toJS(newDoc),
            );
          }
        })
        .then(() => {
          // Set up the sync callback to handle changes from other peers
          // We need to preserve any existing callback that might be set
          const originalOnSync = syncEngine!.options.onSync;

          // Replace the onSync callback with our own implementation
          syncEngine!.options.onSync = async docId => {
            // Only handle changes for our specific document
            logger.info('GOT A SYNC CALLBACK');
            if (docId === resolvedClientId) {
              try {
                // Get the updated document from the sync engine
                const updatedDoc =
                  await syncEngine!.getDocument(resolvedClientId);
                if (updatedDoc) {
                  // Update the Zustand store with the changes
                  handleDocChange(updatedDoc);
                  logger.info('First entry is it remote or no???!: ');
                }
              } catch (error) {
                // Log any errors that occur during the sync process
                logger.error(
                  `Error in sync callback for ${resolvedClientId}:`,
                  error,
                );
              }
            }

            // Call the original callback if it exists
            // This allows multiple documents to be synced independently
            if (originalOnSync) {
              originalOnSync(docId);
            }
          };

          // Mark initialization as complete
          isSyncInit = true;
        })
        .catch(err => {
          // Handle any errors during initialization
          logger.error(
            `Failed to initialize document ${resolvedClientId}:`,
            err,
          );

          // Call the error callback if provided
          if (options.onInitError) {
            // Ensure we always pass an Error object
            options.onInitError(
              err instanceof Error ? err : new Error(String(err)),
            );
          }
        });
    }

    // Configuration for the initialization process
    // Maximum time to wait for the sync engine to become available
    const MAX_INIT_TIME = options.initTimeout ?? 30000; // Default 30 seconds timeout

    // Timer reference for cleanup and cancellation
    let initTimer: ReturnType<typeof setTimeout> | null = null;

    // Track when we started trying to initialize
    let initStartTime = Date.now();

    /**
     * Global Registry Setup
     *
     * This creates a global registry to track callbacks waiting for the sync engine.
     * It's more efficient than having each store instance poll independently.
     *
     * The registry is only created once, even if multiple stores are created.
     */
    if (typeof window !== 'undefined' && !window.__SYNC_ENGINE_REGISTRY__) {
      // Create the registry if it doesn't exist yet
      window.__SYNC_ENGINE_REGISTRY__ = {
        // Array of callbacks to call when the sync engine becomes available
        callbacks: [],

        // Function to notify all waiting callbacks
        notifyCallbacks: function () {
          const syncEngine = getSyncInstance();
          if (syncEngine) {
            // Call each callback with the sync engine instance
            this.callbacks.forEach(cb => cb(syncEngine));
            // Clear the callbacks after notifying them
            this.callbacks = [];
          }
        },
      };

      // Set up a global interval to check for sync engine availability
      const checkInterval = setInterval(() => {
        const syncEngine = getSyncInstance();
        // When the sync engine becomes available, notify all waiting callbacks
        if (syncEngine && window.__SYNC_ENGINE_REGISTRY__) {
          window.__SYNC_ENGINE_REGISTRY__.notifyCallbacks();
          // Stop checking once the engine is available
          clearInterval(checkInterval);
        }
      }, 100); // Check every 100ms
    }

    /**
     * Attempts to initialize sync with a timeout mechanism
     *
     * This function:
     * 1. Checks if we've exceeded the maximum wait time
     * 2. Tries to initialize sync if the engine is available
     * 3. Sets up callbacks or timers to retry if the engine isn't available yet
     *
     * It uses different strategies depending on the environment (browser vs. non-browser)
     */
    const initSyncWithTimeout = () => {
      // STEP 1: Check if we've exceeded the timeout period
      if (Date.now() - initStartTime > MAX_INIT_TIME) {
        // Clean up any pending timers
        if (initTimer) {
          clearTimeout(initTimer);
          initTimer = null;
        }

        // Create and log a timeout error
        const timeoutError = new Error(
          `Sync initialization timed out after ${MAX_INIT_TIME}ms for document ${resolvedClientId}`,
        );
        logger.error(timeoutError.message);

        // Notify the caller about the timeout via the error callback
        if (options.onInitError) {
          options.onInitError(timeoutError);
        }
        return; // Exit the function, giving up on initialization
      }

      // STEP 2: Check if the sync engine is available now
      const syncEngine = getSyncInstance();

      if (syncEngine && !isSyncInit) {
        // CASE 1: Sync engine is available and we haven't initialized yet
        logger.debug(
          `Sync engine available, initializing store for ${resolvedClientId}`,
        );

        // Initialize the sync
        initializeSync();

        // Clean up any pending timers
        if (initTimer) {
          clearTimeout(initTimer);
          initTimer = null;
        }
      } else if (!syncEngine) {
        // CASE 2: Sync engine is not available yet

        // In browser environments, use the registry mechanism
        if (typeof window !== 'undefined' && window.__SYNC_ENGINE_REGISTRY__) {
          // Register a callback to be called when the sync engine becomes available
          window.__SYNC_ENGINE_REGISTRY__.callbacks.push(() => {
            if (!isSyncInit) {
              logger.debug(
                `Sync engine became available, initializing store for ${resolvedClientId}`,
              );
              initializeSync();
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
      // If syncEngine is available but isSyncInit is true, we've already initialized
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

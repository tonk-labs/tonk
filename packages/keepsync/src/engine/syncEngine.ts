import {DocumentData, SyncEngineOptions} from './types.js';
import {
  DocumentId,
  Repo,
  Doc,
  DocHandle,
  DocHandleChangePayload,
} from '@automerge/automerge-repo';
import {WebSocketManager} from './connection/index.js';

import {Storage} from './storage.js';
import {logger} from '../utils/logger.js';
import {throttle} from '../utils/throttle.js';

export class SyncEngine {
  private connection: WebSocketManager;

  private storage: Storage;
  private initialized = false;
  private isOnline = false;
  private repo: Repo | null = null;
  private documentLocks: Map<DocumentId, Promise<void>> = new Map();
  private clientId: string = crypto.randomUUID
    ? crypto.randomUUID()
    : Math.random().toString(36).substring(2, 15) +
      Math.random().toString(36).substring(2, 15);

  private static instance: SyncEngine | null = null;

  /**
   * Get the singleton instance of SyncEngine
   * @param options Options for the SyncEngine
   * @returns The SyncEngine instance
   */
  public static getInstance(): SyncEngine | null {
    if (!SyncEngine.instance) {
      logger.warn('SyncEngine not created yet');
      return null;
    }
    return SyncEngine.instance;
  }

  public static async configureInstance(options: SyncEngineOptions) {
    if (!SyncEngine.instance) {
      SyncEngine.instance = new SyncEngine(options);
      await SyncEngine.instance.init();
      logger.info('SyncEngine created successfully');
    } else {
      logger.warn('SyncEngine instance already exists. Ignoring new options.');
    }

    return SyncEngine.instance;
  }

  /**
   * Reset the singleton instance (mainly for testing purposes)
   */
  public static resetInstance(): void {
    if (SyncEngine.instance) SyncEngine.instance = null;
  }

  private constructor(public options: SyncEngineOptions = {}) {
    // Initialize components
    this.storage = new Storage(options.dbName);

    this.connection = new WebSocketManager(
      {
        url: options.url || 'ws://localhost:4080',
        clientId: this.clientId,
      },
      this.handleConnectionStatusChange.bind(this),
    );

    // With automerge-repo, we don't need message handlers anymore
  }

  async init(): Promise<void> {
    if (this.initialized) return;

    try {
      await this.storage.init();

      // Get the repo from the WebSocketManager
      this.repo = this.connection.getRepo();

      this.initialized = true;
      logger.info('SyncEngine initialized successfully');
    } catch (error) {
      logger.error('Failed to initialize SyncEngine:', error);
      this.initialized = true;
    }
  }

  private handleConnectionStatusChange(isOnline: boolean): void {
    const previousState = this.isOnline;
    this.isOnline = isOnline;

    logger.info(
      `SyncEngine connection status changed to: ${
        isOnline ? 'online' : 'offline'
      }`,
    );

    if (!previousState && isOnline) {
      this.handleReconnection().catch(err => {
        logger.error('Error handling reconnection:', err);
      });
    }
  }

  private async handleReconnection(): Promise<void> {
    try {
      logger.info(
        'Reconnected to server, automerge-repo will handle sync automatically',
      );
      // With automerge-repo, we don't need to manually sync documents
      // The repo will automatically handle syncing when the connection is restored
    } catch (error) {
      logger.error('Error during reconnection:', error);
      this.options.onError?.(error as Error);
    }
  }

  // Public API methods

  // Message handling and sync callbacks are now handled by automerge-repo
  // Use subscribeToDocument() instead for document change notifications

  /**
   * Create a new document with the given ID and initial content
   */
  async createDocument(
    id: DocumentId,
    initialContent: any = {},
  ): Promise<void> {
    if (!this.initialized || !this.repo) {
      throw new Error('SyncEngine not initialized');
    }

    try {
      const newHandle = this.repo.createWithId(id, initialContent);
      // const newHandle = this.repo.find(id);

      // Important: Make a meaningful change to the document
      // This ensures it has heads and won't be seen as "new" during sync
      // Create a new document with the repo
      // const docHandleCache = this.repo.handles;
      // const newHandle = new DocHandle<any>(id, {
      //   isNew: true,
      //   initialValue: initialContent,
      // });
      // // logger.debug('CREATING A DOCUMENT: HEADS', newHandle.heads());
      // // logger.debug('CREATING A DOCUMENT: DOC', await newHandle.doc());
      // // logger.debug('CREATING A DOCUMENT: URL', newHandle.url);

      // docHandleCache[id] = newHandle;
      // this.repo.emit('document', {handle: newHandle, isNew: true});

      // const storageSubsystem = this.repo.storageSubsystem;
      // if (storageSubsystem) {
      //   // Save when the document changes, but no more often than saveDebounceRate.
      //   const saveFn = ({handle, doc}: any) => {
      //     void storageSubsystem.saveDoc(handle.documentId, doc);
      //   };
      //   newHandle.on(
      //     'heads-changed',
      //     throttle(saveFn, this.repo.saveDebounceRate),
      //   );
      // }

      // Register the document with the synchronizer. This advertises our interest in the document.
      // this.repo.

      // newHandle.on('heads-changed', payload => {
      // logger.debug('THE HEADS CHANGED!!!');
      // logger.debug('THE HEADS IN QUESTION', payload.handle.heads());
      // });

      // If a specific ID was requested, we need to use that instead of the auto-generated one
      if (id !== newHandle.documentId) {
        // For now, we'll just log a warning as automerge-repo doesn't support custom IDs directly
        logger.warn(
          `Requested document ID ${id} differs from created ID ${newHandle.documentId}`,
        );
      }

      logger.debug(`Document created with ID ${newHandle.documentId}`);
    } catch (error) {
      logger.error(`Error creating document ${id}:`, error);
      throw error;
    }
  }

  /**
   * Get a document by ID
   */
  async getDocument(id: DocumentId): Promise<any | null> {
    if (!this.repo) {
      throw new Error('Repo not initialized');
    }
    if (!this.initialized || !this.repo) {
      throw new Error('SyncEngine not initialized');
    }

    try {
      // Try to get the document from the repo
      // const docHandle = this.repo.find(id);
      const existingHandle = this.repo.handles[id];
      if (!existingHandle) {
        return null;
      }

      // If the document is unavailable, return null
      if (existingHandle && existingHandle.isUnavailable()) {
        return null;
      }

      // Wait for the document to be ready if it's loading
      if (!existingHandle.isReady()) {
        await existingHandle.whenReady();
      }

      return existingHandle.docSync();
    } catch (error) {
      logger.error(`Error loading document ${id}:`, error);
      return null;
    }
  }

  /**
   * Update a document with the given updater function
   */
  async updateDocument(
    id: DocumentId,
    updater: (doc: any) => void,
  ): Promise<void> {
    if (!this.initialized || !this.repo) {
      throw new Error('SyncEngine not initialized');
    }

    try {
      const docHandle = this.repo!.find(id);

      // Check if the document handle is ready, wait for it if not
      if (!docHandle.isReady()) {
        logger.debug(`Waiting for document to be ready: ${id}`);
        return;
      }

      // Add this to log the document state before update
      logger.info(`Before update, document ${id} state:`, docHandle.docSync());

      docHandle.change(updater);

      this.repo.flush();

      // logger.debug('UDPATING A DOCUMENT: HEADS', docHandle.heads());
      // logger.debug('UPDATING A DOCUMENT: DOC', await docHandle.doc());
      // logger.debug('UPDATING A DOCUMENT: URL', docHandle.url);
      // Add this to log the document state after update
      logger.info(`After update, document ${id} state:`, docHandle.docSync());

      logger.debug(`Document updated with ID ${id}`);
    } catch (error) {
      logger.error(`Error updating document ${id}:`, error);
      throw error;
    }
  }

  /**
   * Get the current connection status
   * @returns True if connected to the server, false otherwise
   */
  isConnected(): boolean {
    return this.isOnline;
  }

  /**
   * Get the initialization status of the sync engine
   * @returns True if the engine is initialized, false otherwise
   */
  isInitialized(): boolean {
    return this.initialized;
  }

  /**
   * Close the sync engine and release resources
   */
  close(): void {
    this.initialized = false;
    this.connection.close();
    this.documentLocks.clear();
    SyncEngine.instance = null;
  }

  /**
   * Subscribe to changes on a specific document
   * @param id Document ID to subscribe to
   * @param callback Function to call when the document changes
   * @returns Unsubscribe function
   */
  subscribeToDocument(
    id: DocumentId,
    callback: (doc: any) => void,
  ): () => void {
    if (!this.repo) {
      logger.warn('Cannot subscribe to document: Repo not initialized');
      return () => {};
    }

    try {
      const docHandle = this.repo.find(id);

      const handleChange = (change: DocHandleChangePayload<any>) => {
        callback(change.doc);
      };

      docHandle.on('change', handleChange);
      return () => docHandle.off('change', handleChange);
    } catch (error) {
      logger.warn(`Cannot subscribe to non-existent document: ${id}`);
      return () => {};
    }
  }

  /**
   * Get the repo instance from the WebSocketManager
   * This allows direct access to the automerge-repo for advanced usage
   */
  getRepo() {
    return this.connection.getRepo();
  }
}

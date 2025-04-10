import {DocumentId, SyncEngineOptions} from './types';
import {WebSocketManager} from './connection';
import {MessageRouter, MessageProtocol} from './messaging';
import {DocumentManager} from './document';
import {Storage} from './storage';
import {logger} from '../utils/logger';

export class SyncEngine {
  private connection: WebSocketManager;
  private messageRouter: MessageRouter;
  private documentManager: DocumentManager;
  private storage: Storage;
  private initialized = false;
  private isOnline = false;
  private pendingDocUpdates: Set<DocumentId> = new Set();
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
    this.documentManager = new DocumentManager(this.storage, options.name);
    this.messageRouter = new MessageRouter(options.name);
    this.connection = new WebSocketManager(
      {url: options.url || 'ws://localhost:8080/sync'},
      this.handleMessage.bind(this),
      this.handleError.bind(this),
      this.handleConnectionStatusChange.bind(this),
    );

    // Register built-in message handlers
    this.messageRouter.registerMessageHandler(
      'client_joined',
      this.handleClientJoined.bind(this),
    );

    this.messageRouter.registerMessageHandler(
      'sync',
      this.handleSyncMessage.bind(this),
    );
  }

  async init(): Promise<void> {
    if (this.initialized) return;

    try {
      await this.storage.init();
      await this.connection.connect();
      this.initialized = true;
      if (this.isOnline) await this.announcePresence();
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
      // Announce presence to other clients
      await this.announcePresence();

      // Sync any documents that were updated offline
      await this.syncPendingUpdates();
    } catch (error) {
      logger.error('Error during reconnection:', error);
      this.options.onError?.(error as Error);
    }
  }

  private async syncPendingUpdates(): Promise<void> {
    if (this.pendingDocUpdates.size > 0) {
      logger.info(
        `Syncing ${this.pendingDocUpdates.size} documents updated while offline`,
      );

      const updates = [...this.pendingDocUpdates];
      this.pendingDocUpdates.clear();

      for (const docId of updates) await this.sendChanges(docId);
    }
  }

  private async handleMessage(message: any): Promise<void> {
    await this.messageRouter.handleMessage(message);
  }

  private handleError(error: Error): void {
    logger.error('SyncEngine error:', error);
    this.options.onError?.(error);
  }

  private async announcePresence(): Promise<void> {
    if (this.isOnline)
      await this.connection.send(
        MessageProtocol.createClientJoinedMessage(this.clientId),
      );
  }

  private async handleClientJoined(message: any): Promise<void> {
    logger.debugWithContext(
      this.options.name || 'SyncEngine',
      `Client joined: ${message.clientId}`,
    );

    // If this is not our own message, respond with our documents
    if (message.clientId !== this.clientId) {
      // Wait a small random time to prevent all clients from responding at once
      const delay = Math.random() * 500;
      await new Promise(resolve => setTimeout(resolve, delay));

      // Send our current state for all documents
      for (const docId of this.documentManager.getAllDocumentIds()) {
        await this.sendChanges(docId);
      }
    }
  }

  private async handleSyncMessage(message: {
    docId: string;
    changes: number[];
  }): Promise<void> {
    try {
      const {docId, changes} = message;
      const result = await this.documentManager.handleIncomingChanges(
        docId,
        changes,
      );

      // Call legacy onSync callback if provided
      if (this.options.onSync) {
        this.options.onSync(docId);
      }

      // If changes were applied, send any pending changes back to ensure bidirectional sync
      if (result.didChange) {
        await this.sendChanges(docId);
      }
    } catch (error) {
      this.handleError(error as Error);
    }
  }

  private async sendChanges(docId: DocumentId): Promise<void> {
    const message = await this.documentManager.generateSyncMessage(docId);
    if (message) {
      await this.connection.send(
        MessageProtocol.createSyncMessage(docId, message),
      );

      // if offline, track doc for sync later
      if (!this.isOnline) {
        this.pendingDocUpdates.add(docId);
      }
    }
  }

  // Public API methods

  /**
   * Register a callback to be called when a document is synced
   */
  registerSyncCallback(callback: (docId: string) => void): void {
    this.messageRouter.registerSyncCallback(callback);
  }

  /**
   * Unregister a sync callback
   */
  unregisterSyncCallback(callback: (docId: string) => void): void {
    this.messageRouter.unregisterSyncCallback(callback);
  }

  /**
   * Register a handler for a specific message type
   */
  registerMessageHandler(
    type: string,
    handler: (data: any) => Promise<void>,
  ): void {
    this.messageRouter.registerMessageHandler(type, handler);
  }

  /**
   * Unregister a message handler
   */
  unregisterMessageHandler(type: string): void {
    this.messageRouter.unregisterMessageHandler(type);
  }

  /**
   * Send a custom message through the WebSocket
   */
  async sendMessage(message: any): Promise<void> {
    await this.connection.send(message);
  }

  /**
   * Create a new document with the given ID and initial content
   */
  async createDocument(
    id: DocumentId,
    initialContent: any = {},
  ): Promise<void> {
    if (!this.initialized) {
      throw new Error('SyncEngine not initialized');
    }

    await this.documentManager.createDocument(id, initialContent);
    await this.sendChanges(id);
  }

  /**
   * Get a document by ID
   */
  async getDocument(id: DocumentId): Promise<any | null> {
    return this.documentManager.getDocument(id);
  }

  /**
   * Update a document with the given updater function
   */
  async updateDocument(
    id: DocumentId,
    updater: (doc: any) => void,
  ): Promise<void> {
    if (!this.initialized) throw new Error('SyncEngine not initialized');

    await this.documentManager.updateDocument(id, updater);

    // if offline, add to pending updates
    if (!this.isOnline) this.pendingDocUpdates.add(id);

    await this.sendChanges(id);
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
    this.messageRouter.clear();
    this.documentManager.clear();
    SyncEngine.instance = null;
  }
}

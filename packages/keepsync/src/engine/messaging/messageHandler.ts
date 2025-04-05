import {MessageHandler, CustomMessage} from '../types.js';
import {logger} from '../../utils/logger.js';

export class MessageRouter {
  private messageHandlers: Map<string, MessageHandler> = new Map();
  private syncCallbacks: Array<(docId: string) => void> = [];
  private name: string;

  constructor(name: string = 'MessageRouter') {
    this.name = name;
  }

  /**
   * Register a callback to be called when a document is synced
   */
  registerSyncCallback(callback: (docId: string) => void): void {
    this.syncCallbacks.push(callback);
  }

  /**
   * Unregister a sync callback
   */
  unregisterSyncCallback(callback: (docId: string) => void): void {
    const index = this.syncCallbacks.indexOf(callback);
    if (index >= 0) {
      this.syncCallbacks.splice(index, 1);
    }
  }

  /**
   * Register a handler for a specific message type
   */
  registerMessageHandler(type: string, handler: MessageHandler): void {
    this.messageHandlers.set(type, handler);
  }

  /**
   * Unregister a message handler
   */
  unregisterMessageHandler(type: string): void {
    this.messageHandlers.delete(type);
  }

  /**
   * Handle an incoming message
   */
  async handleMessage(message: any): Promise<void> {
    if (message instanceof Uint8Array) {
      await this.handleBinaryMessage(message);
      return;
    }

    // Handle different message types
    if (message.type) {
      // This is a custom message
      await this.routeCustomMessage(message);
    } else if (message.docId && message.changes) {
      // This is a document sync message
      await this.routeSyncMessage(message);
    } else {
      logger.debugWithContext(
        this.name,
        'Received message without type or changes:',
        message,
      );
    }
  }

  private async handleBinaryMessage(data: Uint8Array): Promise<void> {
    // Try to decode as JSON first (in case it's a base64-encoded message)
    try {
      const text = new TextDecoder().decode(data);
      const message = JSON.parse(text);
      await this.handleMessage(message);
    } catch (e) {
      // If it's not valid JSON, treat it as raw binary data
      const handler = this.messageHandlers.get('binary');
      if (handler) {
        await handler({data});
      } else {
        logger.debugWithContext(
          this.name,
          'Received binary message without handler',
        );
      }
    }
  }

  private async routeCustomMessage(message: CustomMessage): Promise<void> {
    const handler = this.messageHandlers.get(message.type);
    if (handler) {
      await handler(message);
    } else {
      // Don't warn about unknown message types, just log at debug level
      logger.debugWithContext(
        this.name,
        `Received message type: ${message.type}`,
      );
    }
  }

  private async routeSyncMessage(message: {
    docId: string;
    changes: number[];
  }): Promise<void> {
    logger.debugWithContext(
      this.name,
      `Received changes for ${message.docId}:`,
      message.changes,
    );

    // Notify all sync callbacks about this document
    this.notifySyncCallbacks(message.docId);

    // The actual sync logic will be handled by the DocumentManager
    const syncHandler = this.messageHandlers.get('sync');
    if (syncHandler) {
      await syncHandler(message);
    }
  }

  /**
   * Notify all sync callbacks about a document change
   */
  notifySyncCallbacks(docId: string): void {
    for (const callback of this.syncCallbacks) {
      try {
        callback(docId);
      } catch (error) {
        logger.error('Error in sync callback:', error);
      }
    }
  }

  /**
   * Clear all registered handlers and callbacks
   */
  clear(): void {
    this.syncCallbacks = [];
    this.messageHandlers.clear();
  }
}

import {ConnectionOptions} from '../types.js';
import {logger} from '../../utils/logger.js';

const WebSocketImpl = (() => {
  if (typeof window !== 'undefined' && window.WebSocket) {
    // Browser environment - use native WebSocket
    return window.WebSocket;
  } else {
    // Node.js environment - use ws package
    return require('ws');
  }
})();

export class WebSocketManager {
  private ws: typeof WebSocketImpl | null = null;
  private connectionResolve: () => void;
  private reconnectDelay: number;
  private url: string;
  private onMessage: (data: any) => void;
  private onError: (error: Error) => void;
  private isOnline: boolean = false;
  private reconnectInterval: NodeJS.Timeout | null = null;
  private reconnectAttempts: number = 0;
  private maxReconnectDelay: number = 30000; // max 30 sec between attempts
  private onStatusChange: (isOnline: boolean) => void = () => {};
  private offlineQueue: any[] = [];

  constructor(
    options: ConnectionOptions,
    onMessage: (data: any) => void,
    onError: (error: Error) => void,
    onStatusChange?: (isOnline: boolean) => void,
  ) {
    this.url = options.url;
    this.reconnectDelay = options.reconnectDelay || 1000;
    this.onMessage = onMessage;
    this.onError = onError;
    this.connectionResolve = () => {};

    if (onStatusChange) this.onStatusChange = onStatusChange;
  }

  async connect(): Promise<void> {
    this.connectWebSocket(this.url);
    return Promise.resolve();
  }

  private connectWebSocket(url: string): void {
    try {
      this.ws = new WebSocketImpl(url);

      this.ws.onopen = () => {
        logger.debug('WebSocket connected');
        this.setOnlineStatus(true);
        this.connectionResolve();
        this.reconnectAttempts = 0;
        this.processPendingMessages();
      };

      this.ws.onmessage = async (event: MessageEvent) => {
        try {
          if (event.data instanceof Blob) {
            // Handle binary data
            const arrayBuffer = await event.data.arrayBuffer();
            const uint8Array = new Uint8Array(arrayBuffer);
            this.onMessage(uint8Array);
          } else {
            // Handle JSON messages
            const message = JSON.parse(event.data);
            this.onMessage(message);
          }
        } catch (error) {
          logger.error('WebSocket message error:', error);
          this.onError(error as Error);
        }
      };

      this.ws.onerror = (error: Event) => {
        logger.error('WebSocket error:', error);
        this.setOnlineStatus(false);
        this.onError(error as unknown as Error);
      };

      this.ws.onclose = () => {
        if (this.isOnline) {
          logger.info('WebSocket closed, going to offline mode');
          this.setOnlineStatus(false);
        }

        // exponential backoff for reconnect attempts
        const delay = Math.min(
          this.reconnectDelay * Math.pow(1.5, this.reconnectAttempts),
          this.maxReconnectDelay,
        );
        this.reconnectAttempts++;

        this.scheduleReconnection(delay);
      };
    } catch (error) {
      logger.warn('Failed to create WebSocket:', error);
      this.setOnlineStatus(false);
      this.scheduleReconnection(this.reconnectDelay);
    }
  }

  private scheduleReconnection(delay: number): void {
    // Clear any existing reconnection timer
    if (this.reconnectInterval) clearTimeout(this.reconnectInterval);

    logger.debug(`Scheduling reconnection attempt in ${delay}ms`);
    this.reconnectInterval = setTimeout(() => {
      logger.debug('Attempting to reconnect to WebSocket...');
      this.connectWebSocket(this.url);
    }, delay);
  }

  private setOnlineStatus(isOnline: boolean): void {
    if (this.isOnline !== isOnline) {
      this.isOnline = isOnline;
      logger.info(
        `Connection status changed to: ${isOnline ? 'online' : 'offline'}`,
      );
      this.onStatusChange(isOnline);
    }
  }

  private async processPendingMessages(): Promise<void> {
    if (this.offlineQueue.length > 0) {
      logger.info(`Processing ${this.offlineQueue.length} queued messages`);

      // Create copy of queue and clear original
      const queueToProcess = [...this.offlineQueue];
      this.offlineQueue = [];

      // Send all queued messages
      for (const message of queueToProcess) await this.send(message);

      logger.info('Finished processing queued messages');
    }
  }

  async send(message: any): Promise<void> {
    if (!this.isOnline) {
      logger.debug('Currently offline, queueing message for later');
      this.offlineQueue.push(message);
      return;
    }

    try {
      if (!this.ws || this.ws.readyState !== WebSocketImpl.OPEN) {
        logger.warn('WebSocket not connected, queueing message');
        this.offlineQueue.push(message);
        return;
      }

      if (message instanceof Uint8Array) {
        // Send binary data directly
        this.ws.send(message);
      } else {
        // Send JSON messages as before
        this.ws.send(JSON.stringify(message));
      }
    } catch (error) {
      logger.error('Error sending message:', error);
      // Queue the message if there was an error sending
      this.offlineQueue.push(message);
      this.setOnlineStatus(false);
    }
  }

  close(): void {
    if (this.reconnectInterval) {
      clearTimeout(this.reconnectInterval);
      this.reconnectInterval = null;
    }

    if (this.ws) {
      this.ws.onclose = null;
      this.ws.close();
      this.ws = null;
    }

    this.setOnlineStatus(false);
  }
}

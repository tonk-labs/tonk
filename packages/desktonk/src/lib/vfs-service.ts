import type { DocumentData, JsonValue } from '@tonk/core';
import mime from 'mime';
import type { DocumentContent, VFSWorkerMessage, VFSWorkerResponse } from './types';
import { bytesToString, stringToBytes } from './vfs-utils';

const verbose = () => false;

type ConnectionState = 'disconnected' | 'connecting' | 'open' | 'connected' | 'reconnecting';

export class VFSService {
  private initialized = false;
  private messageId = 0;
  private pendingRequests = new Map<
    string,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
    }
  >();
  private watchers = new Map<string, (documentData: DocumentData) => void>();
  // biome-ignore lint/suspicious/noExplicitAny: Change data structure is dynamic
  private directoryWatchers = new Map<string, (changeData: any) => void>();
  private watcherMetadata = new Map<string, { path: string; type: 'file' | 'directory' }>();
  private connectionState: ConnectionState = 'disconnected';
  private connectionListeners = new Set<(state: ConnectionState) => void>();
  private messageHandler: (event: MessageEvent) => void;

  constructor() {
    this.messageHandler = this.handleMessage.bind(this);
    if (typeof navigator !== 'undefined' && 'serviceWorker' in navigator) {
      navigator.serviceWorker.addEventListener('message', this.messageHandler);

      // Handle controller change (e.g. when SW claims clients)
      navigator.serviceWorker.addEventListener('controllerchange', () => {
        console.log('[VFSService] Controller changed');
        this.notifyConnectionStateChange('connected');
        // If we were already tracking watchers, re-establish them
        if (this.watcherMetadata.size > 0) {
          this.reestablishWatchers();
        }
      });
    } else {
      console.error('[VFSService] Service Worker not supported');
    }
  }

  onConnectionStateChange(listener: (state: ConnectionState) => void): () => void {
    this.connectionListeners.add(listener);
    listener(this.connectionState);
    return () => {
      this.connectionListeners.delete(listener);
    };
  }

  private notifyConnectionStateChange(state: ConnectionState): void {
    this.connectionState = state;
    for (const listener of this.connectionListeners) {
      try {
        listener(state);
      } catch (error) {
        console.error('[VFSService] Error in connection state listener:', error);
      }
    }
  }

  getConnectionState(): ConnectionState {
    return this.connectionState;
  }

  private handleMessage(event: MessageEvent) {
    const response = event.data as VFSWorkerResponse;

    // biome-ignore lint/suspicious/noExplicitAny: Ready message structure is specific
    if ((response as any).type === 'ready') {
      verbose() && console.log('[VFSService] Service Worker is ready');
      return;
    }

    if (response.type === 'disconnected') {
      this.notifyConnectionStateChange('disconnected');
      return;
    }

    if (response.type === 'reconnecting') {
      this.notifyConnectionStateChange('reconnecting');
      return;
    }

    if (response.type === 'reconnected') {
      this.notifyConnectionStateChange('connected');
      return;
    }

    if (response.type === 'watchersReestablished') {
      console.log(`[VFSService] Watchers re-established: ${response.count}`);
      return;
    }

    if (response.type === 'fileChanged' && 'watchId' in response) {
      const callback = this.watchers.get(response.watchId);
      if (callback) {
        callback(response.documentData);
      }
      return;
    }

    if (response.type === 'directoryChanged' && 'watchId' in response) {
      const callback = this.directoryWatchers.get(response.watchId);
      if (callback) {
        callback(response.changeData);
      }
      return;
    }

    if ('id' in response && response.id) {
      const pending = this.pendingRequests.get(response.id);
      if (pending) {
        this.pendingRequests.delete(response.id);
        if (response.success) {
          pending.resolve('data' in response ? response.data : undefined);
        } else {
          pending.reject(new Error(response.error || 'Unknown error'));
        }
      }
    }
  }

  private async ensureServiceWorker(): Promise<ServiceWorker> {
    if (!('serviceWorker' in navigator)) {
      throw new Error('Service Worker is not supported');
    }

    if (navigator.serviceWorker.controller) {
      return navigator.serviceWorker.controller;
    }

    return new Promise((resolve) => {
      const onControllerChange = () => {
        navigator.serviceWorker.removeEventListener('controllerchange', onControllerChange);
        if (navigator.serviceWorker.controller) {
          resolve(navigator.serviceWorker.controller);
        }
      };
      navigator.serviceWorker.addEventListener('controllerchange', onControllerChange);

      // Fallback
      setTimeout(() => {
        if (navigator.serviceWorker.controller) {
          navigator.serviceWorker.removeEventListener('controllerchange', onControllerChange);
          resolve(navigator.serviceWorker.controller);
        }
      }, 5000);
    });
  }

  async connect(): Promise<void> {
    this.notifyConnectionStateChange('connecting');
    await this.ensureServiceWorker();

    try {
      // Check if already initialized/running by asking for server URL or Manifest
      const id = this.generateId();
      await this.sendMessage({ type: 'getServerUrl', id });

      this.initialized = true;
      this.notifyConnectionStateChange('connected');

      if (this.watcherMetadata.size > 0) {
        await this.reestablishWatchers();
      }
    } catch (error) {
      console.log('[VFSService] Service Worker available but not initialized', error);
      // We remain in connected state (to SW), but initialized=false
      this.notifyConnectionStateChange('connected');
    }
  }

  async initialize(manifestUrl: string, wsUrl: string): Promise<void> {
    this.notifyConnectionStateChange('connecting');
    await this.ensureServiceWorker();

    try {
      const id = this.generateId();
      console.log('[VFSService] Sending initializeFromUrl...', { manifestUrl, wsUrl });

      await this.sendMessage({
        type: 'initializeFromUrl',
        id,
        manifestUrl,
        wsUrl,
      });

      console.log('[VFSService] Initialization successful');
      this.initialized = true;
      this.notifyConnectionStateChange('connected');

      if (this.watcherMetadata.size > 0) await this.reestablishWatchers();
    } catch (error) {
      this.notifyConnectionStateChange('disconnected');
      throw new Error(
        `Failed to initialize VFS: ${error instanceof Error ? error.message : String(error)}`
      );
    }
  }

  private generateId(): string {
    return `req_${++this.messageId}`;
  }

  private async sendMessage<T>(message: VFSWorkerMessage & { id: string }): Promise<T> {
    const controller = await this.ensureServiceWorker();

    return new Promise((resolve, reject) => {
      this.pendingRequests.set(message.id, {
        resolve: resolve as (value: unknown) => void,
        reject,
      });

      controller.postMessage(message);

      // Timeout after 30 seconds
      setTimeout(() => {
        if (this.pendingRequests.has(message.id)) {
          this.pendingRequests.delete(message.id);
          reject(new Error('Request timeout'));
        }
      }, 30000);
    });
  }

  async readFile(path: string): Promise<DocumentData> {
    if (!path) {
      console.error('[VFSService] readFile called with no path');
      throw new Error('Path is required for readFile');
    }
    const id = this.generateId();
    return this.sendMessage<DocumentData>({
      type: 'readFile',
      id,
      path,
    });
  }

  async writeFile(path: string, content: DocumentContent, create = false): Promise<void> {
    if (!path) {
      console.error('[VFSService] writeFile called with no path');
      throw new Error('Path is required for writeFile');
    }
    const id = this.generateId();

    return this.sendMessage<void>({
      type: 'writeFile',
      id,
      path,
      content,
      create,
    });
  }

  async writeFileWithBytes(
    path: string,
    content: JsonValue,
    bytes: Uint8Array | string,
    create = false
  ): Promise<void> {
    let bytesData: string;
    if (bytes instanceof Uint8Array) {
      let binaryString = '';
      const chunkSize = 8192;
      for (let i = 0; i < bytes.length; i += chunkSize) {
        const chunk = bytes.subarray(i, i + chunkSize);
        binaryString += String.fromCharCode.apply(null, Array.from(chunk));
      }
      bytesData = btoa(binaryString);
    } else {
      bytesData = bytes;
    }

    return this.writeFile(path, { content, bytes: bytesData }, create);
  }

  async writeStringAsBytes(path: string, stringData: string, create = false): Promise<void> {
    const base64Data = stringToBytes(stringData);
    const mimeType = mime.getType(path) || 'application/octet-stream';

    return this.writeFile(path, { content: { mime: mimeType }, bytes: base64Data }, create);
  }

  async readBytesAsString(path: string): Promise<string> {
    const documentData = await this.readFile(path);

    if (!documentData.bytes) {
      console.warn(`file ${path} was not stored as bytes, returning content instead`);
      return JSON.stringify(documentData.content);
    }

    return bytesToString(documentData);
  }

  async renameFile(oldPath: string, newPath: string): Promise<void> {
    if (!oldPath || !newPath) {
      throw new Error('Both oldPath and newPath are required for renameFile');
    }

    try {
      const doc = await this.readFile(oldPath);
      await this.writeFile(newPath, { content: doc.content, bytes: doc.bytes }, true);
      await this.deleteFile(oldPath);
    } catch (err) {
      console.error('[VFSService] Rename failed', err);
      throw err;
    }
  }

  async deleteFile(path: string): Promise<void> {
    const id = this.generateId();
    return this.sendMessage<void>({
      type: 'deleteFile',
      id,
      path,
    });
  }

  async listDirectory(path: string): Promise<unknown[]> {
    const id = this.generateId();
    return this.sendMessage<unknown[]>({
      type: 'listDirectory',
      id,
      path,
    });
  }

  async exists(path: string): Promise<boolean> {
    if (!path) {
      throw new Error('Path is required for exists check');
    }
    const id = this.generateId();
    return this.sendMessage<boolean>({
      type: 'exists',
      id,
      path,
    });
  }

  async watchFile(path: string, callback: (documentData: DocumentData) => void): Promise<string> {
    const id = this.generateId();
    this.watchers.set(id, callback);
    this.watcherMetadata.set(id, { path, type: 'file' });

    try {
      await this.sendMessage<void>({
        type: 'watchFile',
        id,
        path,
      });
      return id;
    } catch (error) {
      this.watchers.delete(id);
      this.watcherMetadata.delete(id);
      throw error;
    }
  }

  async unwatchFile(watchId: string): Promise<void> {
    this.watchers.delete(watchId);
    this.watcherMetadata.delete(watchId);
    return this.sendMessage<void>({
      type: 'unwatchFile',
      id: watchId,
      path: '',
    });
  }

  async watchDirectory(
    path: string,
    // biome-ignore lint/suspicious/noExplicitAny: Change data structure is dynamic
    callback: (changeData: any) => void
  ): Promise<string> {
    const id = this.generateId();
    this.directoryWatchers.set(id, callback);
    this.watcherMetadata.set(id, { path, type: 'directory' });

    try {
      await this.sendMessage<void>({
        type: 'watchDirectory',
        id,
        path,
      });
      return id;
    } catch (error) {
      this.directoryWatchers.delete(id);
      this.watcherMetadata.delete(id);
      throw error;
    }
  }

  async unwatchDirectory(watchId: string): Promise<void> {
    this.directoryWatchers.delete(watchId);
    this.watcherMetadata.delete(watchId);
    return this.sendMessage<void>({
      type: 'unwatchDirectory',
      id: watchId,
      path: '',
    });
  }

  isInitialized(): boolean {
    return this.initialized;
  }

  private async reestablishWatchers(): Promise<void> {
    console.log('[VFSService] Re-establishing watchers after reconnection...');

    const watcherPromises: Promise<void>[] = [];

    for (const [watchId, metadata] of this.watcherMetadata.entries()) {
      if (metadata.type === 'file') {
        const callback = this.watchers.get(watchId);
        if (callback) {
          const promise = this.sendMessage<void>({
            type: 'watchFile',
            id: watchId,
            path: metadata.path,
          });
          watcherPromises.push(promise);
        }
      } else if (metadata.type === 'directory') {
        const callback = this.directoryWatchers.get(watchId);
        if (callback) {
          const promise = this.sendMessage<void>({
            type: 'watchDirectory',
            id: watchId,
            path: metadata.path,
          });
          watcherPromises.push(promise);
        }
      }
    }

    await Promise.all(watcherPromises);
    console.log(`[VFSService] Re-established ${watcherPromises.length} watchers`);
  }

  destroy(): void {
    this.pendingRequests.clear();
    this.watchers.clear();
    this.directoryWatchers.clear();
    this.initialized = false;
    if (typeof navigator !== 'undefined' && 'serviceWorker' in navigator) {
      navigator.serviceWorker.removeEventListener('message', this.messageHandler);
    }
  }
}

let vfsServiceInstance: VFSService | null = null;

export function getVFSService(): VFSService {
  if (!vfsServiceInstance) {
    vfsServiceInstance = new VFSService();
  }
  return vfsServiceInstance;
}

export function resetVFSService(): void {
  if (vfsServiceInstance) {
    vfsServiceInstance.destroy();
    vfsServiceInstance = null;
  }
}

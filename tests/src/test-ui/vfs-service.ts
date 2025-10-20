import type {
  VFSWorkerMessage,
  VFSWorkerResponse,
  DocumentContent,
} from './types';
import type { DocumentData, JsonValue } from '@tonk/core';
import { bytesToString, stringToBytes } from './vfs-utils';
import mime from 'mime';

interface OperationStats {
  totalOperations: number;
  totalErrors: number;
  recentTimings: number[];
}

export class VFSService {
  private serviceWorker: ServiceWorker | null = null;
  private initialized = false;
  private serviceWorkerReady = false;
  private swReadyPromise: Promise<void> | null = null;
  private swReadyResolve: (() => void) | null = null;
  private messageId = 0;
  private pendingRequests = new Map<
    string,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
    }
  >();
  private watchers = new Map<string, (documentData: DocumentData) => void>();
  private directoryWatchers = new Map<string, (changeData: any) => void>();
  private watcherMetadata = new Map<
    string,
    { path: string; type: 'file' | 'directory' }
  >();

  private operationCount = 0;
  private errorCount = 0;
  private operationTimings: number[] = [];
  private operationListeners = new Set<(stats: OperationStats) => void>();

  constructor() {}

  public getOperationStats(): OperationStats {
    return {
      totalOperations: this.operationCount,
      totalErrors: this.errorCount,
      recentTimings: [...this.operationTimings],
    };
  }

  public onOperationComplete(
    callback: (stats: OperationStats) => void
  ): () => void {
    this.operationListeners.add(callback);
    return () => this.operationListeners.delete(callback);
  }

  private async trackOperation<T>(promise: Promise<T>): Promise<T> {
    const startTime = performance.now();

    return promise
      .then(result => {
        const duration = performance.now() - startTime;
        this.operationCount++;
        this.operationTimings.push(duration);

        if (this.operationTimings.length > 100) {
          this.operationTimings.shift();
        }

        this.notifyListeners();
        return result;
      })
      .catch(error => {
        this.errorCount++;
        this.notifyListeners();
        throw error;
      });
  }

  private notifyListeners(): void {
    const stats = this.getOperationStats();
    this.operationListeners.forEach(callback => callback(stats));
  }

  private async registerServiceWorker(): Promise<void> {
    if (!('serviceWorker' in navigator)) {
      throw new Error('Service workers are not supported in this browser');
    }

    console.log('[VFSService] Registering service worker...');

    const registration = await navigator.serviceWorker.register(
      '/service-worker.js',
      { type: 'module' }
    );

    console.log('[VFSService] Service worker registered:', registration);

    if (registration.installing) {
      console.log('[VFSService] Service worker installing...');
    } else if (registration.waiting) {
      console.log('[VFSService] Service worker waiting...');
    } else if (registration.active) {
      console.log('[VFSService] Service worker active');
    }

    await navigator.serviceWorker.ready;
    console.log('[VFSService] Service worker ready');

    this.serviceWorker =
      registration.active || navigator.serviceWorker.controller;

    if (!this.serviceWorker) {
      throw new Error('Service worker is not available after registration');
    }

    navigator.serviceWorker.addEventListener(
      'message',
      (event: MessageEvent) => {
        const response = event.data as VFSWorkerResponse;
        console.log(
          '[VFSService] 🔍 DEBUG: Received message from service worker:',
          {
            type: response?.type,
            hasId: 'id' in (response || {}),
            id: 'id' in (response || {}) ? (response as any).id : undefined,
            success:
              'success' in (response || {})
                ? (response as any).success
                : undefined,
            response,
          }
        );

        if (response.type === 'ready') {
          console.log(
            '[VFSService] 🔍 DEBUG: Service worker sent ready message'
          );
          return;
        }

        if (response.type === 'swReady') {
          console.log('[VFSService] Service worker ready:', response);
          this.serviceWorkerReady = (response as any).autoInitialized;

          if (this.swReadyResolve) {
            this.swReadyResolve();
            this.swReadyResolve = null;
          }

          if (
            !(response as any).autoInitialized &&
            (response as any).needsBundle
          ) {
            console.log(
              '[VFSService] Service worker needs bundle - will load on initialize()'
            );
          }

          return;
        }

        if (response.type === 'init') {
          console.log(
            '[VFSService] 🔍 DEBUG: Received init response:',
            response
          );
          this.initialized = response.success;
          if (!response.success && response.error) {
            console.error(
              '[VFSService] VFS initialization failed:',
              response.error
            );
          }
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

        if ('id' in response) {
          const pending = this.pendingRequests.get(response.id);
          console.log('[VFSService] 🔍 DEBUG: Handling response with id:', {
            id: (response as any).id,
            hasPending: !!pending,
            pendingRequestsSize: this.pendingRequests.size,
          });

          if (pending) {
            this.pendingRequests.delete(response.id);
            if (response.success) {
              console.log(
                '[VFSService] 🔍 DEBUG: Resolving promise for:',
                (response as any).id
              );
              pending.resolve('data' in response ? response.data : undefined);
            } else {
              console.log(
                '[VFSService] 🔍 DEBUG: Rejecting promise for:',
                (response as any).id,
                response.error
              );
              pending.reject(new Error(response.error || 'Unknown error'));
            }
          } else {
            console.warn(
              '[VFSService] 🔍 DEBUG: No pending request found for id:',
              (response as any).id
            );
          }
        }
      }
    );

    console.log('[VFSService] Service worker setup complete');
  }

  private async waitForServiceWorkerReady(
    timeoutMs: number = 15000
  ): Promise<void> {
    if (this.serviceWorkerReady) {
      return;
    }

    if (!this.swReadyPromise) {
      this.swReadyPromise = new Promise((resolve, reject) => {
        this.swReadyResolve = resolve;

        setTimeout(() => {
          reject(
            new Error(
              'Service worker ready timeout after 15s - likely initialization failed'
            )
          );
        }, timeoutMs);
      });
    }

    return this.swReadyPromise;
  }

  public async waitForReady(timeoutMs: number = 15000): Promise<boolean> {
    if (this.serviceWorkerReady) {
      return true;
    }

    try {
      await this.waitForServiceWorkerReady(timeoutMs);
      return true;
    } catch (error) {
      return false;
    }
  }

  public isReady(): boolean {
    return this.serviceWorkerReady;
  }

  async initialize(manifestUrl: string, wsUrl: string): Promise<void> {
    console.log('[VFSService] 🔍 DEBUG: Starting initialization', {
      manifestUrl,
      wsUrl,
      currentOrigin: window.location.origin,
      serviceWorkerState: this.serviceWorker?.state,
    });

    await this.registerServiceWorker();

    if (!this.serviceWorker) {
      throw new Error('Service worker not initialized');
    }

    // CRITICAL: Wait for service worker to be ready
    console.log('[VFSService] Waiting for service worker to be ready...');
    try {
      await this.waitForServiceWorkerReady(15000);
      console.log('[VFSService] Service worker is ready');
    } catch (error) {
      console.warn(
        '[VFSService] Service worker ready timeout - will retry with bundle load'
      );
    }

    // If service worker auto-initialized, we're done
    if (this.serviceWorkerReady) {
      console.log('[VFSService] Service worker auto-initialized from cache');
      this.initialized = true;
      return;
    }

    console.log(
      '[VFSService] 🔍 DEBUG: Service worker not auto-initialized, fetching manifest...',
      {
        serviceWorkerState: this.serviceWorker.state,
        serviceWorkerScriptURL: this.serviceWorker.scriptURL,
      }
    );

    try {
      console.log('[VFSService] 🔍 DEBUG: About to fetch:', manifestUrl);
      const response = await fetch(manifestUrl);
      console.log('[VFSService] 🔍 DEBUG: Fetch response:', {
        ok: response.ok,
        status: response.status,
        statusText: response.statusText,
        headers: Array.from(response.headers.entries()),
      });

      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }

      const manifest = await response.arrayBuffer();
      console.log('[VFSService] 🔍 DEBUG: Manifest fetched successfully', {
        size: manifest.byteLength,
      });

      const id = this.generateId();

      // Extract server URL from manifestUrl (e.g., http://localhost:8100)
      const manifestUrlObj = new URL(manifestUrl);
      const serverUrl = `${manifestUrlObj.protocol}//${manifestUrlObj.host}`;

      console.log(
        '[VFSService] Sending loadBundle message to service worker...',
        {
          manifestSize: manifest.byteLength,
          wsUrl,
          serverUrl,
          messageId: id,
        }
      );

      const loadBundlePromise = this.sendMessage<void>({
        type: 'loadBundle' as const,
        id,
        bundleBytes: manifest,
        serverUrl,
      } as any);

      console.log('[VFSService] 🔍 DEBUG: Waiting for loadBundle response...');
      await loadBundlePromise;

      console.log('[VFSService] Bundle loaded successfully');
      this.initialized = true;
      this.serviceWorkerReady = true;

      if (this.watcherMetadata.size > 0) {
        await this.reestablishWatchers();
      }
    } catch (error) {
      console.warn(
        '[VFSService] 🔍 DEBUG: Could not fetch manifest from server:',
        {
          error,
          errorMessage: error instanceof Error ? error.message : String(error),
          errorStack: error instanceof Error ? error.stack : undefined,
          manifestUrl,
        }
      );

      console.log(
        '[VFSService] Initializing in offline mode - service worker ready'
      );
      this.initialized = true;
    }
  }

  private generateId(): string {
    return `req_${++this.messageId}`;
  }

  private sendMessage<T>(
    message: VFSWorkerMessage & { id: string }
  ): Promise<T> {
    if (!this.serviceWorker) {
      console.error('[VFSService] Service worker not initialized');
      return Promise.reject(new Error('Service worker not initialized'));
    }

    if (
      !this.serviceWorkerReady &&
      !['loadBundle', 'initializeFromUrl'].includes(message.type)
    ) {
      return Promise.reject(
        new Error('Service worker not ready yet - initialization in progress')
      );
    }

    console.log('[VFSService] 🔍 DEBUG: Sending message to service worker:', {
      type: message.type,
      id: message.id,
      serviceWorkerState: this.serviceWorker.state,
      messageSize:
        'bundleBytes' in message
          ? (message as any).bundleBytes.byteLength
          : 'N/A',
    });

    return new Promise((resolve, reject) => {
      this.pendingRequests.set(message.id, {
        resolve: resolve as (value: unknown) => void,
        reject,
      });

      console.log(
        '[VFSService] 🔍 DEBUG: Added to pending requests, size:',
        this.pendingRequests.size
      );

      this.serviceWorker!.postMessage(message);
      console.log('[VFSService] 🔍 DEBUG: Message posted to service worker');

      setTimeout(() => {
        if (this.pendingRequests.has(message.id)) {
          this.pendingRequests.delete(message.id);
          console.error(
            '[VFSService] 🔍 DEBUG: Request timeout for:',
            message.id
          );
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

  async writeFile(
    path: string,
    content: DocumentContent,
    create = false
  ): Promise<void> {
    if (!path) {
      console.error('[VFSService] writeFile called with no path');
      throw new Error('Path is required for writeFile');
    }
    const id = this.generateId();

    return this.trackOperation(
      this.sendMessage<void>({
        type: 'writeFile',
        id,
        path,
        content,
        create,
      })
    );
  }

  async writeFileWithBytes(
    path: string,
    content: JsonValue,
    bytes: Uint8Array | string,
    create = false
  ): Promise<void> {
    let bytesData: string;

    if (bytes instanceof Uint8Array) {
      const base64 = btoa(
        bytes.reduce((data, byte) => data + String.fromCharCode(byte), '')
      );
      bytesData = base64;
    } else {
      bytesData = bytes;
    }

    return this.writeFile(path, { content, bytes: bytesData }, create);
  }

  async writeStringAsBytes(
    path: string,
    stringData: string,
    create = false
  ): Promise<void> {
    const base64Data = stringToBytes(stringData);
    const mimeType = mime.getType(path) || 'application/octet-stream';

    return this.writeFile(
      path,
      { content: { mime: mimeType }, bytes: base64Data },
      create
    );
  }

  async readBytesAsString(path: string): Promise<string> {
    const documentData = await this.readFile(path);

    if (!documentData.bytes) {
      console.warn(
        `file ${path} was not stored as bytes, returning content instead`
      );
      return JSON.stringify(documentData.content);
    }

    return bytesToString(documentData);
  }

  async deleteFile(path: string): Promise<void> {
    const id = this.generateId();
    return this.trackOperation(
      this.sendMessage<void>({
        type: 'deleteFile',
        id,
        path,
      })
    );
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

  async watchFile(
    path: string,
    callback: (documentData: DocumentData) => void
  ): Promise<string> {
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

  async exportBundle(): Promise<Uint8Array> {
    const id = this.generateId();
    return this.sendMessage<Uint8Array>({
      type: 'toBytes' as any,
      id,
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
          console.log(
            `[VFSService] Re-establishing file watch: ${metadata.path}`
          );
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
          console.log(
            `[VFSService] Re-establishing directory watch: ${metadata.path}`
          );
        }
      }
    }

    await Promise.all(watcherPromises);
    console.log(
      `[VFSService] Re-established ${watcherPromises.length} watchers`
    );
  }

  async destroy(): Promise<void> {
    if (this.serviceWorker) {
      const registration = await navigator.serviceWorker.getRegistration();
      if (registration) {
        await registration.unregister();
      }
      this.serviceWorker = null;
    }
    this.pendingRequests.clear();
    this.initialized = false;
  }
}

let vfsServiceInstance: VFSService | null = null;

export function getVFSService(): VFSService {
  if (!vfsServiceInstance) {
    vfsServiceInstance = new VFSService();
  }
  return vfsServiceInstance;
}

export async function resetVFSService(): Promise<void> {
  if (vfsServiceInstance) {
    await vfsServiceInstance.destroy();
    vfsServiceInstance = null;
  }
}

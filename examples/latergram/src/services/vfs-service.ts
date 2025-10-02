import type { DocumentData, JsonValue } from '@tonk/core';
import mime from 'mime';
import type {
  DocumentContent,
  VFSWorkerMessage,
  VFSWorkerResponse,
} from '../types';
import { bytesToString, stringToBytes } from '../utils/vfs-utils';

// Conditionally import the dev worker
let TonkWorker: any = null;
if (import.meta.env.DEV) {
  import('../tonk-worker.ts?worker').then(module => {
    TonkWorker = module.default;
  });
}

const verbose = () => false;

export class VFSService {
  private worker: Worker | null = null;
  private initialized = false;
  private workerReady = false;
  private workerInitPromise: Promise<void> | null = null;
  private messageId = 0;
  private usingServiceWorkerProxy = false;
  private pendingRequests = new Map<
    string,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
    }
  >();
  private watchers = new Map<string, (documentData: DocumentData) => void>();
  private directoryWatchers = new Map<string, (changeData: any) => void>();

  constructor() {
    this.workerInitPromise = this.initWorker();
  }

  private createServiceWorkerProxy(): Worker {
    // Create a proxy object that mimics the Worker interface but communicates via service worker
    const proxy = {
      onmessage: null as
        | ((event: MessageEvent<VFSWorkerResponse>) => void)
        | null,
      onerror: null as ((error: ErrorEvent) => void) | null,
      onmessageerror: null as ((error: MessageEvent) => void) | null,

      postMessage: (message: VFSWorkerMessage) => {
        if (
          'serviceWorker' in navigator &&
          navigator.serviceWorker.controller
        ) {
          navigator.serviceWorker.controller.postMessage(message);
        } else {
          console.error(
            '[VFSService] Service worker not available for communication'
          );
        }
      },

      terminate: () => {
        // Service worker proxy doesn't need termination
        console.log('[VFSService] Service worker proxy terminated');
      },
    } as Worker;

    // Listen for messages from the service worker
    if ('serviceWorker' in navigator) {
      navigator.serviceWorker.addEventListener('message', event => {
        console.log(
          '[VFSService] Service worker message received:',
          event.data
        );
        if (proxy.onmessage) {
          proxy.onmessage(event as MessageEvent<VFSWorkerResponse>);
        }
      });
    }

    return proxy;
  }

  private async initWorker(): Promise<void> {
    try {
      if (import.meta.env.DEV) {
        // In development, use the local tonk-worker
        if (!TonkWorker) {
          // Wait for dynamic import to complete
          const module = await import('../tonk-worker.ts?worker');
          TonkWorker = module.default;
        }
        console.log('[VFSService] Creating TonkWorker (dev)...');
        this.worker = new TonkWorker();
        this.usingServiceWorkerProxy = false;
      } else {
        // In production, try to use service worker if available
        if ('serviceWorker' in navigator) {
          console.log('[VFSService] Using service worker for communication...');
          // We'll use a custom worker-like object that communicates via service worker
          this.worker = this.createServiceWorkerProxy();
          this.usingServiceWorkerProxy = true;
        } else if (
          typeof Worker !== 'undefined' &&
          (window as any).TonkWorker
        ) {
          console.log('[VFSService] Using runtime-provided worker...');
          // @ts-expect-error
          this.worker = new (window as any).TonkWorker();
          this.usingServiceWorkerProxy = false;
        } else {
          console.warn(
            '[VFSService] No worker available, waiting for runtime to provide one...'
          );
          return;
        }
      }
      console.log('[VFSService] Worker created successfully');
    } catch (error) {
      console.error('Failed to create worker:', error);
      return;
    }

    this.worker.onmessage = (event: MessageEvent<VFSWorkerResponse>) => {
      const response = event.data;
      console.log('[VFSService] Received response from worker:', response);
      verbose() && console.log('Received response from worker:', response);

      if ((response as { type: string }).type === 'ready') {
        verbose() && console.log('Worker is ready!');
        this.workerReady = true;
        return;
      }

      if (response.type === 'init') {
        verbose() && console.log('Received init response:', response);
        this.initialized = response.success;
        if (!response.success && response.error) {
          verbose() &&
            console.error('VFS Worker initialization failed:', response.error);
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
        if (pending) {
          this.pendingRequests.delete(response.id);
          if (response.success) {
            pending.resolve('data' in response ? response.data : undefined);
          } else {
            pending.reject(new Error(response.error || 'Unknown error'));
          }
        }
      }
    };

    this.worker.onerror = error => {
      console.error('VFS Worker error:', error);
    };

    this.worker.onmessageerror = error => {
      console.error('VFS Worker message error:', error);
    };
  }

  async initialize(manifestUrl: string, wsUrl: string): Promise<void> {
    // Wait for worker initialization to complete
    if (this.workerInitPromise) {
      await this.workerInitPromise;
    }

    if (!this.worker) {
      throw new Error('Worker not initialized');
    }

    try {
      const response = await fetch(manifestUrl);
      const manifest = await response.arrayBuffer();

      const message: VFSWorkerMessage = {
        type: 'init',
        manifest,
        wsUrl,
      };

      // Wait for worker to be ready
      console.log('[VFSService] Waiting for worker to be ready...');
      verbose() && console.log('Waiting for worker to be ready...');

      if (this.usingServiceWorkerProxy && 'serviceWorker' in navigator) {
        try {
          console.log(
            '[VFSService] Awaiting navigator.serviceWorker.ready before continuing...'
          );
          await navigator.serviceWorker.ready;
          if (!this.workerReady) {
            console.log(
              '[VFSService] Service worker ready; marking workerReady = true manually.'
            );
            this.workerReady = true;
          }
        } catch (readyError) {
          console.warn(
            '[VFSService] navigator.serviceWorker.ready rejected, continuing anyway:',
            readyError
          );
        }
      }

      let readyPollAttempts = 0;
      await new Promise<void>(resolve => {
        const checkReady = () => {
          readyPollAttempts += 1;
          if (readyPollAttempts % 50 === 0) {
            console.warn(
              '[VFSService] Still waiting for worker ready state...',
              { attempts: readyPollAttempts }
            );
          } else {
            console.log(
              '[VFSService] Checking worker ready state:',
              this.workerReady
            );
          }
          if (this.workerReady) {
            console.log('[VFSService] Worker is ready!');
            resolve();
            return;
          }
          if (readyPollAttempts >= 200) {
            console.error(
              '[VFSService] Worker ready polling exceeded limit, proceeding anyway.'
            );
            resolve();
            return;
          }
          setTimeout(checkReady, 50);
        };
        checkReady();
      });

      console.log('[VFSService] Worker is ready, sending init message...', {
        manifestSize: manifest.byteLength,
        wsUrl,
      });
      this.worker.postMessage(message);
      console.log('[VFSService] Init message sent');

      // Wait for initialization to complete
      return new Promise((resolve, reject) => {
        const checkInit = () => {
          console.log(
            '[VFSService] Checking initialization state:',
            this.initialized
          );
          if (this.initialized) {
            console.log('[VFSService] VFS initialization completed!');
            resolve();
          } else {
            setTimeout(checkInit, 100);
          }
        };
        checkInit();

        // Timeout after 10 seconds
        setTimeout(() => {
          if (!this.initialized) {
            console.error('[VFSService] VFS initialization timeout');
            reject(new Error('VFS initialization timeout'));
          }
        }, 10000);
      });
    } catch (error) {
      throw new Error(
        `Failed to initialize VFS: ${error instanceof Error ? error.message : String(error)}`
      );
    }
  }

  private generateId(): string {
    return `req_${++this.messageId}`;
  }

  private sendMessage<T>(
    message: VFSWorkerMessage & { id: string }
  ): Promise<T> {
    if (!this.worker) {
      console.error('[VFSService] Worker not initialized');
      return Promise.reject(new Error('Worker not initialized'));
    }

    if (!this.initialized) {
      console.error('[VFSService] VFS not initialized');
      return Promise.reject(new Error('VFS not initialized'));
    }

    return new Promise((resolve, reject) => {
      this.pendingRequests.set(message.id, {
        resolve: resolve as (value: unknown) => void,
        reject,
      });
      this.worker!.postMessage(message);

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

    const result = await this.sendMessage<void>({
      type: 'writeFile',
      id,
      path,
      content,
      create,
    });

    return result;
  }

  // Convenience method for writing files with bytes
  async writeFileWithBytes(
    path: string,
    content: JsonValue,
    //either base64 encoded byte data or bytes array
    bytes: Uint8Array | string,
    create = false
  ): Promise<void> {
    // Convert Uint8Array to base64 string if needed
    const bytesData =
      bytes instanceof Uint8Array ? btoa(String.fromCharCode(...bytes)) : bytes;

    return this.writeFile(path, { content, bytes: bytesData }, create);
  }

  // Convenience method for writing string data as bytes
  async writeStringAsBytes(
    path: string,
    stringData: string,
    create = false
  ): Promise<void> {
    // Convert string to UTF-8 bytes then to base64
    const base64Data = stringToBytes(stringData);

    // Determine MIME type from file path
    const mimeType = mime.getType(path) || 'application/octet-stream';

    return this.writeFile(
      path,
      { content: { mime: mimeType }, bytes: base64Data },
      create
    );
  }

  // Convenience method for reading string data from bytes
  async readBytesAsString(path: string): Promise<string> {
    const documentData = await this.readFile(path);

    if (!documentData.bytes) {
      console.warn(
        `file ${path} was not stored as bytes, returning content instead`
      );
      return JSON.stringify(documentData.content);
    }

    // Decode base64 to bytes then to UTF-8 string
    return bytesToString(documentData);
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

  async watchFile(
    path: string,
    callback: (documentData: DocumentData) => void
  ): Promise<string> {
    const id = this.generateId();
    this.watchers.set(id, callback);

    try {
      await this.sendMessage<void>({
        type: 'watchFile',
        id,
        path,
      });
      return id;
    } catch (error) {
      this.watchers.delete(id);
      throw error;
    }
  }

  async unwatchFile(watchId: string): Promise<void> {
    this.watchers.delete(watchId);
    return this.sendMessage<void>({
      type: 'unwatchFile',
      id: watchId,
      path: '', // Not used for unwatch
    });
  }

  async watchDirectory(
    path: string,
    callback: (changeData: any) => void
  ): Promise<string> {
    const id = this.generateId();
    this.directoryWatchers.set(id, callback);

    try {
      await this.sendMessage<void>({
        type: 'watchDirectory',
        id,
        path,
      });
      return id;
    } catch (error) {
      this.directoryWatchers.delete(id);
      throw error;
    }
  }

  async unwatchDirectory(watchId: string): Promise<void> {
    this.directoryWatchers.delete(watchId);
    return this.sendMessage<void>({
      type: 'unwatchDirectory',
      id: watchId,
      path: '', // Not used for unwatch
    });
  }

  isInitialized(): boolean {
    return this.initialized;
  }

  destroy(): void {
    if (this.worker) {
      this.worker.terminate();
      this.worker = null;
    }
    this.pendingRequests.clear();
    this.watchers.clear();
    this.directoryWatchers.clear();
    this.initialized = false;
  }
}

// Singleton instance
let vfsServiceInstance: VFSService | null = null;

export function getVFSService(): VFSService {
  if (!vfsServiceInstance) {
    vfsServiceInstance = new VFSService();
  }
  return vfsServiceInstance;
}

// Function to reset the singleton (useful for testing or cleanup)
export function resetVFSService(): void {
  if (vfsServiceInstance) {
    vfsServiceInstance.destroy();
    vfsServiceInstance = null;
  }
}

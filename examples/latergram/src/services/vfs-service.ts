import type { VFSWorkerMessage, VFSWorkerResponse } from '../types';
import TonkWorker from '../tonk-worker.ts?worker';

const verbose = () => false;

export class VFSService {
  private worker: Worker | null = null;
  private initialized = false;
  private workerReady = false;
  private messageId = 0;
  private pendingRequests = new Map<
    string,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
    }
  >();
  private watchers = new Map<string, (content: string) => void>();
  private directoryWatchers = new Map<string, (changeData: any) => void>();

  constructor() {
    this.initWorker();
  }

  private initWorker() {
    try {
      this.worker = new TonkWorker();
    } catch (error) {
      verbose() && console.error('Failed to create worker:', error);
      return;
    }

    this.worker.onmessage = (event: MessageEvent<VFSWorkerResponse>) => {
      const response = event.data;
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
          verbose() && console.error('VFS Worker initialization failed:', response.error);
        }
        return;
      }

      if (response.type === 'fileChanged' && 'watchId' in response) {
        const callback = this.watchers.get(response.watchId);
        if (callback) {
          callback(response.content);
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
      verbose() && console.error('VFS Worker error:', error);
    };

    this.worker.onmessageerror = error => {
      verbose() && console.error('VFS Worker message error:', error);
    };
  }

  async initialize(manifestUrl: string, wsUrl: string): Promise<void> {
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
      verbose() && console.log('Waiting for worker to be ready...');
      await new Promise<void>(resolve => {
        const checkReady = () => {
          if (this.workerReady) {
            resolve();
          } else {
            setTimeout(checkReady, 10);
          }
        };
        checkReady();
      });

      verbose() && console.log('Worker is ready, sending init message...');
      this.worker.postMessage(message);
      verbose() && console.log('Init message sent');

      // Wait for initialization to complete
      return new Promise((resolve, reject) => {
        const checkInit = () => {
          if (this.initialized) {
            resolve();
          } else {
            setTimeout(checkInit, 100);
          }
        };
        checkInit();

        // Timeout after 10 seconds
        setTimeout(() => {
          if (!this.initialized) {
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
      return Promise.reject(new Error('Worker not initialized'));
    }

    if (!this.initialized) {
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

  async readFile(path: string): Promise<string> {
    if (!path) {
      throw new Error('Path is required for readFile');
    }
    const id = this.generateId();
    return this.sendMessage<string>({
      type: 'readFile',
      id,
      path,
    });
  }

  async writeFile(
    path: string,
    content: string,
    create = false
  ): Promise<void> {
    if (!path) {
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
    callback: (content: string) => void
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

// Types for VFS service worker communication
interface VFSWorkerMessage {
  type:
    | 'init'
    | 'readFile'
    | 'writeFile'
    | 'deleteFile'
    | 'listDirectory'
    | 'exists'
    | 'watchFile'
    | 'unwatchFile';
  id?: string;
  manifest?: ArrayBuffer;
  wsUrl?: string;
  path?: string;
  content?: string;
  create?: boolean;
}

interface VFSWorkerResponse {
  type: string;
  id?: string;
  success: boolean;
  data?: unknown;
  error?: string;
  watchId?: string;
  content?: string;
}

export class VFSService {
  private serviceWorker: ServiceWorker | null = null;
  private initialized = false;
  private messageId = 0;
  private pendingRequests = new Map<
    string,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
    }
  >();
  private watchers = new Map<string, (content: string) => void>();

  constructor() {
    this.initServiceWorker();
  }

  private async initServiceWorker() {
    try {
      // Wait for service worker to be ready
      if (!navigator.serviceWorker) {
        throw new Error('Service Worker not supported');
      }

      // Get the active service worker
      const registration = await navigator.serviceWorker.ready;
      this.serviceWorker = registration.active;

      if (!this.serviceWorker) {
        throw new Error('No active service worker found');
      }

      // Listen for messages from service worker
      navigator.serviceWorker.addEventListener(
        'message',
        (event: MessageEvent<VFSWorkerResponse>) => {
          const response = event.data;
          console.log('Received response from service worker:', response);

          if (
            response.type === 'fileChanged' &&
            'watchId' in response &&
            response.watchId
          ) {
            const callback = this.watchers.get(response.watchId);
            if (callback && response.content) {
              callback(response.content);
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
      );
    } catch (error) {
      console.error('Failed to initialize service worker:', error);
    }
  }

  async initialize(): Promise<void> {
    // The service worker is already initialized and handling VFS operations
    // We just need to wait for our service worker connection to be ready
    if (!this.serviceWorker) {
      throw new Error('Service Worker not initialized');
    }

    console.log('VFS Service ready to use existing service worker');
    this.initialized = true;
  }

  private generateId(): string {
    return `req_${++this.messageId}`;
  }

  private sendMessage<T>(
    message: VFSWorkerMessage & { id: string }
  ): Promise<T> {
    if (!this.serviceWorker) {
      return Promise.reject(new Error('Service Worker not initialized'));
    }

    if (!this.initialized) {
      return Promise.reject(new Error('VFS not initialized'));
    }

    return new Promise((resolve, reject) => {
      this.pendingRequests.set(message.id, {
        resolve: resolve as (value: unknown) => void,
        reject,
      });
      if (this.serviceWorker) {
        this.serviceWorker.postMessage(message);
      }

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
    const id = this.generateId();
    return this.sendMessage<void>({
      type: 'writeFile',
      id,
      path,
      content,
      create,
    });
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

  isInitialized(): boolean {
    return this.initialized;
  }

  destroy(): void {
    // We don't terminate the service worker as it's shared
    // Just clean up our internal state
    this.serviceWorker = null;
    this.pendingRequests.clear();
    this.watchers.clear();
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

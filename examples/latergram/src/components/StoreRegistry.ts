export interface StoreMetadata {
  id: string;
  name: string;
  filePath: string;
  created: Date;
  modified: Date;
  status: 'loading' | 'success' | 'error';
  error?: string;
}

export interface ProxiedStore {
  id: string;
  metadata: StoreMetadata;
  original: any; // The zustand store hook
  proxy: any; // The proxied store hook with hot reload
  version: number;
}

class StoreRegistry {
  private stores: Map<string, ProxiedStore> = new Map();
  private updateCallbacks: Map<string, Set<() => void>> = new Map();
  private contextUpdateCallbacks: Set<() => void> = new Set();

  register(
    id: string,
    storeHook: any,
    metadata?: Partial<StoreMetadata>
  ): ProxiedStore {
    const now = new Date();
    const storeMetadata: StoreMetadata = {
      id,
      name: metadata?.name || `Store-${id}`,
      filePath: metadata?.filePath || `/src/stores/${id}.ts`,
      created: metadata?.created || now,
      modified: metadata?.modified || now,
      status: 'success',
      ...metadata,
    };

    const proxied = this.createProxy(id, storeHook, storeMetadata);
    this.stores.set(id, proxied);
    this.notifyContextUpdate();

    return proxied;
  }

  createStore(name: string, filePath?: string): string {
    const id = this.generateId();
    const metadata: StoreMetadata = {
      id,
      name,
      filePath: filePath || `/src/stores/${id}.ts`,
      created: new Date(),
      modified: new Date(),
      status: 'loading',
    };

    // Create a dummy store hook that returns empty state
    const dummyStore = () => ({});
    const proxied = this.createProxy(id, dummyStore, metadata);
    this.stores.set(id, proxied);
    this.notifyContextUpdate();

    return id;
  }

  updateMetadata(id: string, updates: Partial<StoreMetadata>): void {
    const existing = this.stores.get(id);
    if (existing) {
      existing.metadata = {
        ...existing.metadata,
        ...updates,
        modified: new Date(),
      };
      this.notifyUpdate(id);
    }
  }

  getAllStores(): ProxiedStore[] {
    return Array.from(this.stores.values());
  }

  getStoresByStatus(status: StoreMetadata['status']): ProxiedStore[] {
    return this.getAllStores().filter(
      store => store.metadata.status === status
    );
  }

  deleteStore(id: string): boolean {
    const deleted = this.stores.delete(id);
    if (deleted) {
      this.updateCallbacks.delete(id);
      this.notifyContextUpdate();
    }
    return deleted;
  }

  private generateId(): string {
    return `store-${Date.now()}-${Math.random().toString(36).substring(2, 11)}`;
  }

  update(
    id: string,
    newStoreHook: any,
    status: StoreMetadata['status'] = 'success',
    error?: string
  ): void {
    const existing = this.stores.get(id);
    if (!existing) {
      return;
    }

    existing.original = newStoreHook;
    existing.version++;
    existing.metadata.modified = new Date();
    existing.metadata.status = status;
    if (error) {
      existing.metadata.error = error;
    } else {
      delete existing.metadata.error;
    }

    this.notifyUpdate(id);
    this.notifyContextUpdate();

    // If the store just became successful, trigger a re-render of components
    if (status === 'success') {
      // Notify all components that stores have been updated
      setTimeout(() => {
        this.notifyContextUpdate();
      }, 50);
    }
  }

  onUpdate(id: string, callback: () => void): () => void {
    if (!this.updateCallbacks.has(id)) {
      this.updateCallbacks.set(id, new Set());
    }

    const callbacks = this.updateCallbacks.get(id)!;
    callbacks.add(callback);

    return () => {
      callbacks.delete(callback);
      if (callbacks.size === 0) {
        this.updateCallbacks.delete(id);
      }
    };
  }

  onContextUpdate(callback: () => void): () => void {
    this.contextUpdateCallbacks.add(callback);
    return () => {
      this.contextUpdateCallbacks.delete(callback);
    };
  }

  private notifyUpdate(id: string): void {
    const callbacks = this.updateCallbacks.get(id);
    if (callbacks) {
      callbacks.forEach(cb => cb());
    }
  }

  private notifyContextUpdate(): void {
    this.contextUpdateCallbacks.forEach(cb => cb());
  }

  private createProxy(
    id: string,
    storeHook: any,
    metadata: StoreMetadata
  ): ProxiedStore {
    const registry = this;

    // Create a proxied store hook that always uses the latest version
    const ProxyStoreHook = function (...args: any[]) {
      const current = registry.stores.get(id);
      const StoreHook = current?.original || storeHook;
      return StoreHook(...args);
    };

    // Preserve hook properties
    Object.defineProperty(ProxyStoreHook, 'name', {
      value: metadata.name,
      configurable: true,
    });

    return {
      id,
      metadata,
      original: storeHook,
      proxy: ProxyStoreHook,
      version: 1,
    };
  }

  clear(): void {
    this.stores.clear();
    this.updateCallbacks.clear();
    this.notifyContextUpdate();
  }

  getStore(id: string): ProxiedStore | undefined {
    return this.stores.get(id);
  }

  getStoreByName(name: string): ProxiedStore | undefined {
    return Array.from(this.stores.values()).find(
      store => store.metadata.name === name
    );
  }

  getStoreByFilePath(filePath: string): ProxiedStore | undefined {
    return Array.from(this.stores.values()).find(
      store => store.metadata.filePath === filePath
    );
  }
}

export const storeRegistry = new StoreRegistry();

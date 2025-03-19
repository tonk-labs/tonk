// Type definitions for global sync engine registry
interface SyncEngineRegistry {
  callbacks: Array<(syncEngine: any) => void>;
  notifyCallbacks: () => void;
}

declare global {
  interface Window {
    __SYNC_ENGINE_REGISTRY__?: SyncEngineRegistry;
  }
}

export {};

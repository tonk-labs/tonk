import {
  NetworkAdapter,
  StorageAdapterInterface,
} from '@automerge/automerge-repo';

export interface SyncEngineOptions {
  storage?: StorageAdapterInterface;
  networkAdapters?: NetworkAdapter[];
}

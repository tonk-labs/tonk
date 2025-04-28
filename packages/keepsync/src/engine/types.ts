import {
  NetworkAdapterInterface,
  StorageAdapterInterface,
} from '@tonk/automerge-repo';

export interface SyncEngineOptions {
  storage?: StorageAdapterInterface;
  networkAdapters?: NetworkAdapterInterface[];
}

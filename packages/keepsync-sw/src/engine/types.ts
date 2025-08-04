export interface SyncEngineConfig {
  url: string;
  storage: SyncParameterConfig<'nodefs' | 'indexeddb' | undefined>;
  network: SyncParameterConfig<'ws' | undefined>;
}

export type SyncParameterConfig<T> = {
  type: T;
  metadata: any;
};

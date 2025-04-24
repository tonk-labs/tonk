export type DocumentId = string;

export interface BundleServerConfig {
  bundleName: string;
  port: number;
  bundlePath: string;
  hasServices: boolean;
  verbose?: boolean;
}

export interface BundleServerInfo {
  id: string;
  port: number;
  bundleName: string;
  status: 'running' | 'stopped';
  startedAt?: Date;
}

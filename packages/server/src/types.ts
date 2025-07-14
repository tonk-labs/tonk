import {RootNode} from './rootNode.js';
export type DocumentId = string;

export interface BundleServerConfig {
  bundleName: string;
  port: number;
  bundlePath: string;
  hasServices: boolean;
  verbose?: boolean;
  rootNode: RootNode;
}

export interface BundleServerInfo {
  id: string;
  port?: number;
  route?: string;
  bundleName: string;
  status: 'running' | 'stopped';
  startedAt?: Date;
  url?: string;
}

import {RootNode} from './rootNode.js';
export type DocumentId = string;

export interface ApiService {
  prefix: string;
  baseUrl: string;
  requiresAuth?: boolean;
  authType?: 'apikey' | 'bearer' | 'basic' | 'query';
  authHeaderName?: string;
  authEnvVar?: string;
  authQueryParamName?: string;
}

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
  port: number;
  bundleName: string;
  status: 'running' | 'stopped';
  startedAt?: Date;
}

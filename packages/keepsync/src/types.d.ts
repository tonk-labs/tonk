/// <reference types="vite/client" />

import { DocumentId } from './engine';

// Type definitions for global sync engine registry
interface SyncEngineRegistry {
  callbacks: Array<(syncEngine: any) => void>;
  notifyCallbacks: () => void;
}

declare global {
  interface Window {
    __SYNC_ENGINE_REGISTRY__?: SyncEngineRegistry;
    electronAPI: {
      writeDocument: (documentId: DocumentId, doc: Uint8Array) => Promise<void>;
      readDocument: (documentId: DocumentId) => Promise<Uint8Array>;
      launchApp: (projectPath: string) => Promise<string>;
      createApp: (projectName: string) => Promise<void>;
      runShell: (dirPath: string) => Promise<void>;
      closeShell: () => Promise<void>;
      openExternal: (link: string) => Promise<void>;
      getConfig: () => Promise<Config>;
      init: (homePath: string) => Promise<void>;
      clearConfig: () => Promise<void>;
      copyHubTemplate: () => Promise<void>;
      fetchRegistry: () => Promise<{ success: boolean; data: Registry }>;
      readBinary: (filePath: string) => Promise<Uint8Array>;
      readFile: (filePath: string) => Promise<string>;
      writeFile: (filePath: string, content: string) => Promise<void>;
      ls: (dirPath: string) => Promise<FileDescription[]>;
      platformSensitiveJoin: (paths: string[]) => Promise<string>;
      showOpenDialog: (options: {
        properties: string[];
      }) => Promise<{ canceled: boolean; filePaths: string[] }>;
      startFileWatching: () => Promise<boolean>;
      stopFileWatching: () => Promise<boolean>;
      runServer: (restart: boolean) => Promise<void>;

      getDocumentsPath: () => string;
      installIntegration: (
        integrationLink: string
      ) => Promise<{ success: boolean; data?: string; error?: string }>;
      getInstalledIntegrations: () => Promise<{
        success: boolean;
        data?: InstalledIntegration[];
        error?: string;
      }>;
    };
    require: (module: string) => any;
  }
}

export {};

// Add type declaration for the electron API
declare global {
  interface Window {
    electronAPI: {
      launchApp: (projectPath: string) => Promise<string>;
      launchAppDev: (projectPath: string) => Promise<string>;
      createApp: (projectName: string) => Promise<void>;
      runShell: (dirPath: string) => Promise<void>;
      closeShell: () => Promise<void>;
      openExternal: (link: string) => Promise<void>;
      openUrlInElectron: (url: string) => Promise<boolean>;
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
    };
    require: (module: string) => any;
  }
}

export type FileDescription = {
  name: string;
  isDirectory: boolean;
  isFile: boolean;
  isSymlink: boolean;
};

export type Config = {
  homePath: string;
};

export type FileChangeEvent = {
  type: "add" | "change" | "unlink" | "addDir" | "unlinkDir";
  path: string;
};

export type Integration = {
  name: string;
  link: string;
  description: string;
  isInstalled: boolean;
  version?: string;
};

export type Registry = {
  packages: Integration[];
};

export type InstalledIntegration = {
  name: string;
  version: string;
  description: string;
  dependencies: Record<string, string>;
  devDependencies: Record<string, string>;
};

// Add type declaration for the electron API
declare global {
  interface Window {
    electronAPI: {
      launchApp: (projectPath: string) => Promise<void>;
      getConfig: () => Promise<Config>;
      init: (homePath: string) => Promise<void>;
      showOpenDialog: (options: {
        properties: string[];
      }) => Promise<{ canceled: boolean; filePaths: string[] }>;
    };
    require: (module: string) => any;
  }
}

export type Config = {
  homePath: string;
};

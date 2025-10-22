// Re-export service worker types
export * from '../types';

// Screen state enum
export enum ScreenState {
  LOADING = 'loading',
  BOOT = 'boot',
  ERROR = 'error',
  PROMPT = 'prompt',
  SPLASH = 'splash',
}

// App types
export interface App {
  name: string;
  slug?: string;
  status?: string;
}

// Dialog state types
export interface DialogState {
  confirmation: {
    isOpen: boolean;
    appName: string;
  };
  share: {
    isOpen: boolean;
    isLoading: boolean;
    qrCodeUrl: string | null;
    shareUrl: string | null;
    error: string | null;
  };
  name: {
    isOpen: boolean;
    defaultName: string;
    hasLoadedBundles: boolean;
  };
  downloadSpinner: {
    isOpen: boolean;
  };
}

// Context types
export interface TonkContextValue {
  screenState: ScreenState;
  setScreenState: (state: ScreenState) => void;
  loadingMessage: string;
  setLoadingMessage: (message: string) => void;
  errorMessage: string;
  setErrorMessage: (message: string) => void;
  availableApps: App[];
  setAvailableApps: (apps: App[]) => void;
  selectedAppIndex: number;
  setSelectedAppIndex: (index: number) => void;
  showError: (message: string) => void;
  showLoadingScreen: (message?: string) => void;
  showBootMenu: () => void;
  showPromptScreen: () => void;
  showSplashScreen: () => void;
}

export interface DialogContextValue {
  dialogs: DialogState;
  openConfirmationDialog: (appName: string) => void;
  closeConfirmationDialog: () => void;
  openShareDialog: () => void;
  closeShareDialog: () => void;
  updateShareDialog: (updates: Partial<DialogState['share']>) => void;
  openNameDialog: (defaultName: string, hasLoadedBundles: boolean) => void;
  closeNameDialog: () => void;
  openDownloadSpinner: () => void;
  closeDownloadSpinner: () => void;
}

// Service Worker message types
export interface ServiceWorkerMessage {
  type: string;
  id?: string;
  success?: boolean;
  data?: any;
  error?: string;
  [key: string]: any;
}

// Hook return types
export interface UseServiceWorkerReturn {
  sendMessage: <T = any>(message: any) => Promise<T>;
  queryAvailableApps: () => Promise<string[]>;
  confirmBoot: (appSlug: string) => Promise<void>;
  downloadTonk: () => Promise<void>;
  createNewTonk: (tonkName: string, hasLoadedBundles: boolean) => Promise<void>;
  getServerUrl: () => Promise<string>;
  shareAsUrl: () => Promise<{ shareUrl: string; qrCodeUrl: string }>;
}

export interface UseDragDropReturn {
  processTonkFile: (file: File) => Promise<void>;
}

export interface UseKeyboardNavReturn {
  // This hook uses effects internally, no return value needed
}

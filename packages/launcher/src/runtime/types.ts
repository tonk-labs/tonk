// Re-export service worker types from launcher kernel types
export type { VFSWorkerMessage, VFSWorkerResponse } from '../launcher/types';

// Screen state enum
export enum ScreenState {
  LOADING = 'loading',
  ERROR = 'error',
}

// Context types
export interface TonkContextValue {
  screenState: ScreenState;
  loadingMessage: string;
  errorMessage: string;
  showError: (message: string) => void;
  showLoadingScreen: (message?: string) => void;
}

// Service Worker message types (generic wrapper)
export interface ServiceWorkerMessage {
  type: string;
  id?: string;
  success?: boolean;
  // biome-ignore lint/suspicious/noExplicitAny: Generic data payload
  data?: any;
  error?: string;
  // biome-ignore lint/suspicious/noExplicitAny: Allow other properties
  [key: string]: any;
}

// Hook return types
export interface UseServiceWorkerReturn {
  // biome-ignore lint/suspicious/noExplicitAny: Generic return type
  sendMessage: <T = any>(message: any) => Promise<T>;
  queryAvailableApps: () => Promise<string[]>;
  confirmBoot: (appSlug: string) => Promise<void>;
}

// Types for VFS Worker messaging

export type VFSWorkerMessage =
  | { type: 'init'; wsUrl: string }
  | { type: 'readFile'; id: string; path: string }
  | {
      type: 'writeFile';
      id: string;
      path: string;
      content: string;
      bytes: string; //base64 encoded string
      create: boolean;
    }
  | { type: 'deleteFile'; id: string; path: string }
  | { type: 'listDirectory'; id: string; path: string }
  | { type: 'exists'; id: string; path: string }
  | { type: 'watchFile'; id: string; path: string }
  | { type: 'unwatchFile'; id: string };

export type VFSWorkerResponse =
  | { type: 'ready' }
  | { type: 'init'; success: boolean; error?: string }
  | {
      type: 'readFile';
      id: string;
      success: boolean;
      data?: string;
      error?: string;
    }
  | { type: 'writeFile'; id: string; success: boolean; error?: string }
  | { type: 'deleteFile'; id: string; success: boolean; error?: string }
  | {
      type: 'listDirectory';
      id: string;
      success: boolean;
      data?: any;
      error?: string;
    }
  | {
      type: 'exists';
      id: string;
      success: boolean;
      data?: boolean;
      error?: string;
    }
  | { type: 'watchFile'; id: string; success: boolean; error?: string }
  | { type: 'unwatchFile'; id: string; success: boolean; error?: string }
  | { type: 'fileChanged'; watchId: string; content: string };

// DocumentWatcher interface
export interface DocumentWatcher {
  stop: () => void;
}

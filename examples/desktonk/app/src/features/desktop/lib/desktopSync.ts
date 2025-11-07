/**
 * Cross-tab desktop synchronization using BroadcastChannel.
 * Provides real-time updates when files are added/removed from desktop,
 * even across multiple tabs or users.
 */

const CHANNEL_NAME = 'desktop-sync';

export type DesktopSyncMessage =
  | { type: 'file-added'; path: string }
  | { type: 'file-removed'; path: string }
  | { type: 'files-changed' }
  | { type: 'refresh' };

type DesktopSyncCallback = (message: DesktopSyncMessage) => void;

class DesktopSyncManager {
  private channel: BroadcastChannel | null = null;
  private callbacks: Set<DesktopSyncCallback> = new Set();

  constructor() {
    if (typeof BroadcastChannel !== 'undefined') {
      this.channel = new BroadcastChannel(CHANNEL_NAME);
      this.channel.onmessage = (event) => {
        console.log('[DesktopSync] Received message:', event.data);
        this.callbacks.forEach(callback => callback(event.data));
      };
    } else {
      console.warn('[DesktopSync] BroadcastChannel not supported, sync disabled');
    }
  }

  /**
   * Broadcast a message to all tabs
   */
  broadcast(message: DesktopSyncMessage): void {
    if (this.channel) {
      console.log('[DesktopSync] Broadcasting:', message);
      this.channel.postMessage(message);
    }
  }

  /**
   * Subscribe to sync messages
   */
  subscribe(callback: DesktopSyncCallback): () => void {
    this.callbacks.add(callback);
    return () => {
      this.callbacks.delete(callback);
    };
  }

  /**
   * Clean up resources
   */
  destroy(): void {
    if (this.channel) {
      this.channel.close();
      this.channel = null;
    }
    this.callbacks.clear();
  }
}

// Singleton instance
export const desktopSync = new DesktopSyncManager();

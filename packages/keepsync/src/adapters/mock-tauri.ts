/**
 * Mock Tauri API for testing IrohNetworkAdapter
 */
export class MockTauriAPI {
  private listeners: Map<string, Array<(event: any) => void>> = new Map();
  private peers: Map<string, any> = new Map();

  constructor() {
    // Auto-emit ready event after initialization
    setTimeout(() => this.emitEvent('iroh_ready', {}), 100);
  }

  /**
   * Mock invoke function
   */
  async invoke(cmd: string, args?: any): Promise<any> {
    switch (cmd) {
      case 'init_iroh':
        return Promise.resolve();

      case 'start_discovery':
        // Simulate discovering a peer after a delay
        setTimeout(() => {
          this.emitEvent('peer_discovered', {
            peerId: 'mock-peer-123',
            bundleId: args.bundleId,
            nodeAddr: 'mock-addr',
          });
        }, 500);
        return Promise.resolve();

      case 'connect_to_peer':
        const peerId = args.peerId;
        this.peers.set(peerId, { connected: true });

        // Simulate connection success
        setTimeout(() => {
          this.emitEvent('peer_connected', { peerId });
        }, 200);
        return Promise.resolve();

      case 'send_automerge_message':
        // In a real test, this could be connected to another adapter
        return Promise.resolve();

      case 'disconnect_all_peers':
        this.peers.forEach((_, peerId) => {
          this.emitEvent('peer_disconnected', { peerId });
        });
        this.peers.clear();
        return Promise.resolve();

      default:
        return Promise.reject(new Error(`Unknown command: ${cmd}`));
    }
  }

  /**
   * Mock listen function
   */
  async listen(
    event: string,
    handler: (event: any) => void
  ): Promise<() => void> {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, []);
    }

    const handlers = this.listeners.get(event)!;
    handlers.push(handler);

    // Return unlisten function
    return () => {
      const index = handlers.indexOf(handler);
      if (index !== -1) {
        handlers.splice(index, 1);
      }
    };
  }

  /**
   * Emit an event to all listeners
   */
  emitEvent(event: string, payload: any) {
    const handlers = this.listeners.get(event);
    if (handlers) {
      handlers.forEach(handler => {
        handler({ payload });
      });
    }
  }

  /**
   * Simulate receiving a message from a peer
   */
  simulateMessage(peerId: string, message: any) {
    this.emitEvent('automerge_message', {
      peerId,
      message,
    });
  }

  /**
   * Simulate a peer discovery
   */
  simulatePeerDiscovery(peerId: string, bundleId: string) {
    this.emitEvent('peer_discovered', {
      peerId,
      bundleId,
      nodeAddr: `mock-addr-${peerId}`,
    });
  }

  /**
   * Get the list of connected peers
   */
  getConnectedPeers(): string[] {
    return Array.from(this.peers.entries())
      .filter(([_, info]) => info.connected)
      .map(([peerId]) => peerId);
  }
}

/**
 * Install mock Tauri API in global window object
 */
export function installMockTauri(): MockTauriAPI {
  if (typeof window === 'undefined') {
    (global as any).window = {};
  }

  const mockTauri = new MockTauriAPI();
  (window as any).__TAURI__ = {
    invoke: (cmd: string, args?: any) => mockTauri.invoke(cmd, args),
    event: {
      listen: (event: string, handler: (event: any) => void) =>
        mockTauri.listen(event, handler),
    },
  };

  return mockTauri;
}

/**
 * Remove mock Tauri API from global window object
 */
export function uninstallMockTauri() {
  if (typeof window !== 'undefined') {
    delete (window as any).__TAURI__;
  }
}

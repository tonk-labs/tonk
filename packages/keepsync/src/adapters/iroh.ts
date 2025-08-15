import {
  NetworkAdapter,
  Message,
  PeerId,
  PeerMetadata,
} from '@automerge/automerge-repo';
import { logger } from '../utils/logger.js';

/**
 * Message type for Automerge messages sent through Iroh
 */
interface IrohAutomergeMessage {
  type: string;
  data: number[];
}

/**
 * Peer information from Iroh discovery
 */
interface IrohPeerInfo {
  peerId: string;
  bundleId: string;
  nodeAddr?: string;
}

/**
 * Tauri API interface for Iroh commands
 */
interface TauriAPI {
  invoke: (cmd: string, args?: any) => Promise<any>;
  listen: (event: string, handler: (event: any) => void) => Promise<() => void>;
}

/**
 * Network adapter for Iroh P2P connectivity
 * Bridges between Iroh (Rust) and Keepsync (TypeScript) using Tauri IPC
 */
export class IrohNetworkAdapter extends NetworkAdapter {
  private peers: Map<PeerId, boolean> = new Map();
  private unlisteners: Array<() => void> = [];
  private tauri: TauriAPI | null = null;
  private ready: boolean = false;
  private readyPromise: Promise<void>;
  private readyResolve: ((value: void) => void) | null = null;

  constructor() {
    super();
    this.readyPromise = new Promise<void>(resolve => {
      this.readyResolve = resolve;
    });
    this.initializeTauriAPI();
  }

  /**
   * Initialize Tauri API if available
   */
  private async initializeTauriAPI() {
    // Check if running in Tauri environment
    if (typeof window !== 'undefined' && (window as any).__TAURI__) {
      // Check if we have mock Tauri (for testing)
      const tauriGlobal = (window as any).__TAURI__;
      if (tauriGlobal.invoke && tauriGlobal.event?.listen) {
        // Use mock/test Tauri API
        this.tauri = {
          invoke: tauriGlobal.invoke,
          listen: tauriGlobal.event.listen,
        };
        await this.setupListeners();
      } else {
        try {
          // Use real Tauri API
          const { invoke } = await import('@tauri-apps/api/tauri');
          const { listen } = await import('@tauri-apps/api/event');
          this.tauri = { invoke, listen };
          await this.setupListeners();
        } catch (error) {
          logger.error('Failed to import Tauri API:', error);
          // Mark as ready even if Tauri is not available
          this.ready = true;
          if (this.readyResolve) {
            this.readyResolve();
          }
        }
      }
    } else {
      logger.warn(
        'IrohNetworkAdapter: Not running in Tauri environment, adapter will be inactive'
      );
      // Mark as ready immediately when not in Tauri environment
      this.ready = true;
      if (this.readyResolve) {
        this.readyResolve();
      }
    }
  }

  /**
   * Set up event listeners for Iroh P2P events
   */
  private async setupListeners() {
    if (!this.tauri) return;

    try {
      // Listen for Automerge messages from Iroh connections
      const unlistenMessage = await this.tauri.listen(
        'automerge_message',
        (event: any) => {
          const { peerId, message } = event.payload;
          logger.debug('Received Automerge message from peer:', peerId);

          // Convert array back to Uint8Array
          const data = message.data
            ? new Uint8Array(message.data)
            : new Uint8Array();

          // Forward to Automerge-repo for processing
          this.emit('message', {
            senderId: peerId as PeerId,
            targetId: this.peerId,
            type: message.type,
            data: data,
          } as Message);
        }
      );
      this.unlisteners.push(unlistenMessage);

      // Listen for peer discovery events
      const unlistenDiscovery = await this.tauri.listen(
        'peer_discovered',
        (event: any) => {
          const peerInfo: IrohPeerInfo = event.payload;
          logger.info('Peer discovered:', peerInfo.peerId);

          this.peers.set(peerInfo.peerId as PeerId, false);

          // Emit peer-candidate event for Automerge-repo
          this.emit('peer-candidate', {
            peerId: peerInfo.peerId as PeerId,
            peerMetadata: {
              // Bundle info would go in custom data, not in standard PeerMetadata
              isEphemeral: false,
            },
          });

          // Auto-connect to discovered peers
          this.connectToPeer(peerInfo.peerId as PeerId);
        }
      );
      this.unlisteners.push(unlistenDiscovery);

      // Listen for peer connection status
      const unlistenConnected = await this.tauri.listen(
        'peer_connected',
        (event: any) => {
          const { peerId } = event.payload;
          logger.info('Peer connected:', peerId);

          this.peers.set(peerId as PeerId, true);

          // Note: No need to emit additional events here, peer-candidate already handled discovery
        }
      );
      this.unlisteners.push(unlistenConnected);

      // Listen for peer disconnection
      const unlistenDisconnected = await this.tauri.listen(
        'peer_disconnected',
        (event: any) => {
          const { peerId } = event.payload;
          logger.info('Peer disconnected:', peerId);

          this.peers.delete(peerId as PeerId);

          // Emit peer-disconnected event
          this.emit('peer-disconnected', {
            peerId: peerId as PeerId,
          });
        }
      );
      this.unlisteners.push(unlistenDisconnected);

      // Listen for network ready status
      const unlistenReady = await this.tauri.listen('iroh_ready', () => {
        logger.info('Iroh network adapter ready');
        this.ready = true;
        if (this.readyResolve) {
          this.readyResolve();
        }
      });
      this.unlisteners.push(unlistenReady);
    } catch (error) {
      logger.error('Failed to set up Iroh event listeners:', error);
    }
  }

  /**
   * Check if the adapter is ready
   */
  isReady(): boolean {
    return this.ready;
  }

  /**
   * Wait for the adapter to be ready
   */
  whenReady(): Promise<void> {
    return this.readyPromise;
  }

  /**
   * Connect to the network with a given peer ID
   */
  connect(peerId: PeerId, peerMetadata?: PeerMetadata) {
    this.peerId = peerId;
    this.peerMetadata = peerMetadata;

    if (this.tauri) {
      // Initialize Iroh with our peer ID
      this.tauri
        .invoke('init_iroh', { peerId })
        .then(() => {
          logger.info('Iroh initialized with peer ID:', peerId);
        })
        .catch(error => {
          logger.error('Failed to initialize Iroh:', error);
        });
    } else {
      // If not in Tauri environment, mark as ready immediately
      this.ready = true;
      if (this.readyResolve) {
        this.readyResolve();
      }
    }
  }

  /**
   * Send a message to a peer
   */
  send(message: Message) {
    if (!this.tauri) {
      logger.debug('Cannot send message: not in Tauri environment');
      return;
    }

    if (!message.targetId) {
      logger.warn('Cannot send message: no target peer ID');
      return;
    }

    // Convert Uint8Array to regular array for IPC
    const messageData: IrohAutomergeMessage = {
      type: message.type || '',
      data: message.data ? Array.from(message.data) : [],
    };

    // Send Automerge sync message through Iroh
    this.tauri
      .invoke('send_automerge_message', {
        targetId: message.targetId,
        message: messageData,
      })
      .catch(error => {
        logger.error(
          'Failed to send message to peer:',
          message.targetId,
          error
        );
      });
  }

  /**
   * Connect to a specific peer
   */
  private async connectToPeer(peerId: PeerId) {
    if (!this.tauri) return;

    try {
      await this.tauri.invoke('connect_to_peer', { peerId });
      logger.info('Initiated connection to peer:', peerId);
    } catch (error) {
      logger.error('Failed to connect to peer:', peerId, error);
    }
  }

  /**
   * Disconnect from the network
   */
  disconnect() {
    if (this.tauri) {
      // Disconnect from all peers
      this.tauri
        .invoke('disconnect_all_peers')
        .then(() => {
          logger.info('Disconnected from all peers');
        })
        .catch(error => {
          logger.error('Failed to disconnect from peers:', error);
        });
    }

    // Clear local state
    this.peers.clear();
    this.ready = false;

    // Remove all event listeners
    this.unlisteners.forEach(unlisten => unlisten());
    this.unlisteners = [];
  }

  /**
   * Start discovery for a specific bundle
   */
  async startDiscovery(bundleId: string) {
    if (!this.tauri) {
      logger.warn('Cannot start discovery: not in Tauri environment');
      return;
    }

    try {
      await this.tauri.invoke('start_discovery', { bundleId });
      logger.info('Started P2P discovery for bundle:', bundleId);
    } catch (error) {
      logger.error('Failed to start discovery:', error);
    }
  }

  /**
   * Get the list of connected peers
   */
  getConnectedPeers(): PeerId[] {
    return Array.from(this.peers.entries())
      .filter(([_, connected]) => connected)
      .map(([peerId]) => peerId);
  }
}

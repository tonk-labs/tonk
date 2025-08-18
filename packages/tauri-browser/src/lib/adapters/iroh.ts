import {
  NetworkAdapter,
  Message,
  PeerId,
  PeerMetadata,
} from '@automerge/automerge-repo';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface AutomergeMessage {
  message_type: string;
  data: number[];
}

export interface PeerInfo {
  peer_id: string;
  bundle_id: string;
  connected: boolean;
}

export interface TonkServiceInfo {
  bundle_id: string;
  node_id: string;
  protocol_version: number;
  port: number;
  capabilities: string[];
  instance_name: string;
  addresses: string[];
  discovered_at: number;
}

export interface DiscoveryConfig {
  enabled: boolean;
  announce_interval: number;
  browse_interval: number;
  service_ttl: number;
  max_peers: number;
  interface?: string;
}

export interface ConnectionAttempt {
  peer_id: string;
  attempts: number;
  last_attempt: string;
  next_attempt: string;
  backoff_duration: number;
  max_attempts: number;
  connected: boolean;
}

export interface ConnectionStatus {
  [peerId: string]: ConnectionAttempt;
}

export class IrohNetworkAdapter extends NetworkAdapter {
  private peers: Map<PeerId, boolean> = new Map();
  private discoveredPeers: Map<PeerId, TonkServiceInfo> = new Map();
  private _isReady = false;
  private readyPromise: Promise<void>;
  private resolveReady?: () => void;

  constructor() {
    super();
    this.readyPromise = new Promise(resolve => {
      this.resolveReady = resolve;
    });
    this.setupListeners();
  }

  isReady(): boolean {
    return this._isReady;
  }

  whenReady(): Promise<void> {
    return this.readyPromise;
  }

  private setReady(): void {
    if (!this._isReady) {
      this._isReady = true;
      if (this.resolveReady) {
        this.resolveReady();
      }
    }
  }

  private setupListeners() {
    // Listen for Automerge messages from Iroh connections
    listen('automerge_message', (event: any) => {
      const { peerId, message } = event.payload;
      console.log('Received automerge message from peer:', peerId);

      // Convert number array back to Uint8Array
      const data = new Uint8Array(message.data);

      // Forward to Automerge-repo for processing
      this.emit('message', {
        senderId: peerId as PeerId,
        targetId: this.peerId || ('' as PeerId),
        type: message.message_type,
        data: data,
      });
    });

    // Listen for peer discovery events
    listen('peer_discovered', (event: any) => {
      const { peerId, bundleId, addresses, port, capabilities } = event.payload;
      console.log('Peer discovered:', peerId, bundleId);

      // Store discovered peer info
      const serviceInfo: TonkServiceInfo = {
        bundle_id: bundleId,
        node_id: peerId,
        protocol_version: 1,
        port: port || 0,
        capabilities: capabilities || ['sync'],
        instance_name: `tonk-${bundleId.slice(0, 8)}`,
        addresses: addresses || [],
        discovered_at: Date.now(),
      };

      this.discoveredPeers.set(peerId, serviceInfo);
      this.peers.set(peerId, false); // Not connected yet

      // Auto-connect to discovered peers
      this.connectToPeer(peerId);
    });

    // Listen for peer lost events
    listen('peer_lost', (event: any) => {
      const { peerId } = event.payload;
      console.log('Peer lost:', peerId);

      this.discoveredPeers.delete(peerId);
      this.peers.delete(peerId);
    });

    // Listen for connection status
    listen('peer_connected', (event: any) => {
      const { peerId } = event.payload;
      console.log('Peer connected:', peerId);

      this.peers.set(peerId, true);
    });

    // Listen for connection failures
    listen('peer_connection_failed', (event: any) => {
      const { peerId, reason } = event.payload;
      console.log(`Peer connection failed: ${peerId}, reason: ${reason}`);
    });

    // Listen for retry attempts
    listen('peer_retry_attempt', (event: any) => {
      const { peerId } = event.payload;
      console.log('Retrying connection to peer:', peerId);
    });

    // Listen for connection exhaustion
    listen('peer_connection_exhausted', (event: any) => {
      const { peerId } = event.payload;
      console.log(`Connection attempts exhausted for peer: ${peerId}`);
    });

    // Listen for discovery events
    listen('discovery_event', (event: any) => {
      const { type, service } = event.payload;
      console.log(`Discovery event: ${type}`, service);
    });

    // Listen for P2P ready event
    listen('p2p_ready', (event: any) => {
      console.log('P2P system ready:', event.payload);
      this.setReady();
    });

    // Listen for disconnection events
    listen('all_peers_disconnected', () => {
      console.log('All peers disconnected');
      this.peers.clear();
      this.discoveredPeers.clear();
    });
  }

  async initialize(): Promise<void> {
    try {
      await invoke('initialize_p2p');
      console.log('P2P system initialized');
    } catch (error) {
      console.error('Failed to initialize P2P system:', error);
      throw error;
    }
  }

  async startDiscovery(bundleId: string): Promise<void> {
    try {
      await invoke('start_discovery', { bundleId });
      console.log('Started discovery for bundle:', bundleId);
    } catch (error) {
      console.error('Failed to start discovery:', error);
      throw error;
    }
  }

  connect(peerId: PeerId, peerMetadata?: PeerMetadata): void {
    this.peerId = peerId;
    this.peerMetadata = peerMetadata;
    console.log('Network adapter connected with peerId:', peerId);
  }

  send(message: Message): void {
    if (!this._isReady) {
      console.warn('IrohNetworkAdapter not ready, dropping message');
      return;
    }

    // Ensure message.data exists and convert to array
    const data = message.data ? Array.from(message.data) : [];

    // Send Automerge sync messages through Iroh
    invoke('send_automerge_message', {
      targetId: message.targetId,
      message: {
        message_type: message.type,
        data,
      },
    }).catch((error: any) => {
      console.error('Failed to send automerge message:', error);
    });
  }

  private connectToPeer(peerId: PeerId): Promise<void> {
    console.log('Connecting to peer:', peerId);
    return invoke('connect_to_peer', { peerId });
  }

  disconnect(): void {
    console.log('Disconnecting all peers');
    this.peers.clear();
    this._isReady = false;
    invoke('disconnect_all_peers').catch((error: any) => {
      console.error('Failed to disconnect peers:', error);
    });
  }

  async getConnectedPeers(): Promise<PeerInfo[]> {
    try {
      return await invoke('get_connected_peers');
    } catch (error) {
      console.error('Failed to get connected peers:', error);
      return [];
    }
  }

  async getDiscoveredPeers(): Promise<TonkServiceInfo[]> {
    try {
      return await invoke('get_discovered_peers');
    } catch (error) {
      console.error('Failed to get discovered peers:', error);
      return [];
    }
  }

  async getNodeId(): Promise<string | null> {
    try {
      return await invoke('get_node_id');
    } catch (error) {
      console.error('Failed to get node ID:', error);
      return null;
    }
  }

  async getConnectionStatus(peerId: string): Promise<ConnectionAttempt | null> {
    try {
      return await invoke('get_connection_status', { peerId });
    } catch (error) {
      console.error('Failed to get connection status:', error);
      return null;
    }
  }

  async getAllConnectionAttempts(): Promise<ConnectionStatus> {
    try {
      return await invoke('get_all_connection_attempts');
    } catch (error) {
      console.error('Failed to get connection attempts:', error);
      return {};
    }
  }

  async resetConnectionAttempts(peerId: string): Promise<void> {
    try {
      await invoke('reset_connection_attempts', { peerId });
    } catch (error) {
      console.error('Failed to reset connection attempts:', error);
    }
  }

  async toggleLocalDiscovery(enabled: boolean): Promise<void> {
    try {
      await invoke('toggle_local_discovery', { enabled });
    } catch (error) {
      console.error('Failed to toggle local discovery:', error);
    }
  }

  async restartDiscovery(): Promise<void> {
    try {
      await invoke('restart_discovery');
    } catch (error) {
      console.error('Failed to restart discovery:', error);
    }
  }

  async getP2PStatus(): Promise<any> {
    try {
      return await invoke('get_p2p_status');
    } catch (error) {
      console.error('Failed to get P2P status:', error);
      return null;
    }
  }
}

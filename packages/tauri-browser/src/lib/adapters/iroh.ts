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

export class IrohNetworkAdapter extends NetworkAdapter {
  private peers: Map<PeerId, boolean> = new Map();
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
      const { peerId, bundleId } = event.payload;
      console.log('Peer discovered:', peerId, bundleId);

      this.peers.set(peerId, true);
      // Auto-connect to discovered peers
      this.connectToPeer(peerId);
    });

    // Listen for connection status
    listen('peer_connected', (event: any) => {
      const { peerId } = event.payload;
      console.log('Peer connected:', peerId);

      this.peers.set(peerId, true);
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

  async getNodeId(): Promise<string | null> {
    try {
      return await invoke('get_node_id');
    } catch (error) {
      console.error('Failed to get node ID:', error);
      return null;
    }
  }
}

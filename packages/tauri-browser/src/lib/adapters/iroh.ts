import {
  NetworkAdapter,
  Message,
  PeerId,
  PeerMetadata,
  RepoMessage,
} from '@automerge/automerge-repo';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

// Handshake message types following MessageChannel adapter pattern
// These are used internally for the handshake protocol
interface ArriveMessage {
  type: 'arrive';
  senderId: PeerId;
  peerMetadata: PeerMetadata;
  targetId?: never;
}

interface WelcomeMessage {
  type: 'welcome';
  senderId: PeerId;
  targetId: PeerId;
  peerMetadata: PeerMetadata;
}

interface LeaveMessage {
  type: 'leave';
  senderId: PeerId;
}

// Union type for all handshake messages (used internally)
export type HandshakeMessage = ArriveMessage | WelcomeMessage | LeaveMessage;

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
  private ready = false;
  private readyPromise: Promise<void>;
  private readyResolver?: () => void;
  private remotePeerId?: PeerId;

  constructor() {
    super();
    this.readyPromise = new Promise(resolve => {
      this.readyResolver = resolve;
    });
    this.setupListeners();
  }

  isReady(): boolean {
    return this.ready;
  }

  whenReady(): Promise<void> {
    return this.readyPromise;
  }

  private setReady(): void {
    if (!this.ready) {
      this.ready = true;
      if (this.readyResolver) {
        this.readyResolver();
      }
    }
  }

  private setupListeners() {
    // Listen for Automerge messages from Iroh connections
    listen('automerge_message', (event: any) => {
      const { peerId, message } = event.payload;
      const senderId = peerId as PeerId;

      console.log(
        'Received message from peer:',
        peerId,
        'type:',
        message.type,
        'data length:',
        message.data ? message.data.length : 0
      );

      // Handle handshake messages
      switch (message.type) {
        case 'arrive': {
          // Peer is announcing themselves, send welcome back
          const peerMetadata = message.peerMetadata || {};
          console.log('Received arrive message from:', senderId);

          // Send welcome message back using invoke directly for handshake
          invoke('send_automerge_message', {
            targetId: senderId,
            message: {
              type: 'welcome',
              senderId: this.peerId,
              targetId: senderId,
              peerMetadata: this.peerMetadata || {},
            },
          }).catch((error: any) => {
            console.error('Failed to send welcome message:', error);
          });

          // Announce the connection
          this.announceConnection(senderId, peerMetadata);
          break;
        }

        case 'welcome': {
          // Peer is welcoming us after our arrive message
          const peerMetadata = message.peerMetadata || {};
          console.log('Received welcome message from:', senderId);

          // Announce the connection
          this.announceConnection(senderId, peerMetadata);
          break;
        }

        case 'leave': {
          // Peer is disconnecting
          console.log('Received leave message from:', senderId);
          if (this.remotePeerId === senderId) {
            this.emit('peer-disconnected', { peerId: senderId });
            this.emit('close');
          }
          break;
        }

        default: {
          // Regular Automerge sync message
          const messageForRepo: Message = {
            ...message,
            senderId,
            targetId: this.peerId!,
            type: message.type,
          };

          console.log('RECEIVED MESSAGE:', message);

          // Add documentId if present (required for sync and request messages)
          // if (message.documentId) {
          //   messageForRepo.documentId = message.documentId;
          // }

          // Convert data if present
          if (message.data && message.data.length > 0) {
            messageForRepo.data = new Uint8Array(message.data);
          }

          console.log('Emitting sync message to automerge-repo:', {
            ...messageForRepo,
            data: messageForRepo.data
              ? `[Uint8Array ${messageForRepo.data.length} bytes]`
              : undefined,
          });

          this.emit('message', messageForRepo);
          break;
        }
      }
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

      // Emit peer-disconnected event to notify Automerge repo
      this.emit('peer-disconnected', { peerId });
    });

    // Listen for connection status
    listen('peer_connected', (event: any) => {
      const { peerId } = event.payload;
      console.log('Peer connected:', peerId);

      this.peers.set(peerId, true);

      // Send arrive message to the newly connected peer to start handshake
      if (this.peerId) {
        console.log('Sending arrive message to newly connected peer:', peerId);
        invoke('send_automerge_message', {
          targetId: peerId,
          message: {
            type: 'arrive',
            senderId: this.peerId,
            peerMetadata: this.peerMetadata || {},
          },
        }).catch((error: any) => {
          console.error('Failed to send arrive message to new peer:', error);
        });
      }
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
      // Don't mark as ready here - wait for handshake
    });

    // Listen for disconnection events
    listen('all_peers_disconnected', () => {
      console.log('All peers disconnected');

      // Emit peer-disconnected for each connected peer
      for (const peerId of this.peers.keys()) {
        this.emit('peer-disconnected', { peerId });
      }

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
    console.log(
      'Network adapter connect() called with peerId:',
      peerId,
      'metadata:',
      peerMetadata
    );

    // Send arrive message to all connected peers (if any are connected yet)
    this.sendArrive();

    // Mark as ready after a timeout if no handshake completes
    setTimeout(() => {
      if (!this.ready) {
        console.log(
          'Handshake timeout reached, marking adapter as ready anyway'
        );
        this.setReady();
      }
    }, 100);
  }

  send(message: RepoMessage): void {
    // Only handle regular Automerge repo messages here
    // Handshake messages are sent directly via invoke

    const messageToSend: any = {
      ...message,
      type: message.type,
      senderId: message.senderId || this.peerId,
      targetId: message.targetId,
    };

    // Add documentId if present (required for sync and request messages)
    // if ('documentId' in message && message.documentId) {
    //   messageToSend.documentId = message.documentId;
    // }

    // Convert binary data if present
    if ('data' in message && message.data) {
      // Convert Uint8Array to regular array for JSON serialization
      messageToSend.data = Array.from(message.data);
    }

    console.log('Automerge adapter sending message:', {
      type: messageToSend.type,
      senderId: messageToSend.senderId,
      targetId: messageToSend.targetId,
      documentId: messageToSend.documentId,
      dataLength: messageToSend.data ? messageToSend.data.length : 0,
    });

    console.log('RAW MESSAGE:', message);
    console.log('SENDING MESSAGE:', messageToSend);

    // Send through Iroh
    invoke('send_automerge_message', {
      targetId: messageToSend.targetId,
      message: messageToSend,
    }).catch((error: any) => {
      console.error('Failed to send automerge message:', error);
    });
  }

  disconnect(): void {
    console.log('Disconnecting all peers');

    // Send leave message to all connected peers
    if (this.peerId) {
      this.peers.forEach((connected, peerId) => {
        if (connected) {
          invoke('send_automerge_message', {
            targetId: peerId,
            message: {
              type: 'leave',
              senderId: this.peerId,
            },
          }).catch((error: any) => {
            console.error('Failed to send leave message:', error);
          });
        }
      });
    }

    this.peers.clear();
    this.ready = false;
    invoke('disconnect_all_peers').catch((error: any) => {
      console.error('Failed to disconnect peers:', error);
    });
  }

  private sendArrive(): void {
    // Send arrive message to announce ourselves to all connected peers
    console.log('sendArrive called, checking connected peers...');
    const connectedPeers = Array.from(this.peers.entries()).filter(
      ([_, connected]) => connected
    );

    if (connectedPeers.length === 0) {
      console.log('No connected peers yet, skipping arrive broadcast');
      return;
    }

    console.log(
      `Sending arrive message to ${connectedPeers.length} connected peers`
    );
    connectedPeers.forEach(([peerId, _]) => {
      console.log('Sending arrive message to peer:', peerId);
      invoke('send_automerge_message', {
        targetId: peerId,
        message: {
          type: 'arrive',
          senderId: this.peerId,
          peerMetadata: this.peerMetadata || {},
        },
      }).catch((error: any) => {
        console.error('Failed to send arrive message:', error);
      });
    });
  }

  private announceConnection(peerId: PeerId, peerMetadata: PeerMetadata): void {
    console.log(
      'Announcing connection to Automerge repo for peer:',
      peerId,
      'with metadata:',
      peerMetadata
    );
    this.remotePeerId = peerId;
    this.setReady();
    this.emit('peer-candidate', { peerId, peerMetadata });
    console.log('Emitted peer-candidate event for peer:', peerId);
  }

  private connectToPeer(peerId: PeerId): Promise<void> {
    console.log('Connecting to peer:', peerId);
    return invoke('connect_to_peer', { peerId });
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

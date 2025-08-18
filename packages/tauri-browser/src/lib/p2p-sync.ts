import { Repo, PeerId } from '@automerge/automerge-repo';
import { IndexedDBStorageAdapter } from '@automerge/automerge-repo-storage-indexeddb';
import { IrohNetworkAdapter } from './adapters/iroh.js';

export class P2PSync {
  private repo?: Repo;
  private networkAdapter?: IrohNetworkAdapter;

  async initialize(bundleId: string): Promise<void> {
    // Create and initialize the Iroh network adapter
    this.networkAdapter = new IrohNetworkAdapter();
    await this.networkAdapter.initialize();
    await this.networkAdapter.startDiscovery(bundleId);

    // Get our node ID for the peer ID
    const nodeId = await this.networkAdapter.getNodeId();
    if (!nodeId) {
      throw new Error('Failed to get node ID from P2P system');
    }

    // Initialize the Automerge repo with P2P networking
    this.repo = new Repo({
      peerId: nodeId as PeerId,
      network: [this.networkAdapter],
      storage: new IndexedDBStorageAdapter('tonk-p2p'),
    });

    // Wait for the network to be ready
    await this.networkAdapter.whenReady();

    console.log('P2P sync initialized for bundle:', bundleId);
  }

  getRepo(): Repo {
    if (!this.repo) {
      throw new Error('P2PSync not initialized. Call initialize() first.');
    }
    return this.repo;
  }

  async getConnectedPeers() {
    if (!this.networkAdapter) {
      return [];
    }
    return this.networkAdapter.getConnectedPeers();
  }

  async getDiscoveredPeers() {
    if (!this.networkAdapter) {
      return [];
    }
    return this.networkAdapter.getDiscoveredPeers();
  }

  async getConnectionStatus(peerId: string) {
    if (!this.networkAdapter) {
      return null;
    }
    return this.networkAdapter.getConnectionStatus(peerId);
  }

  async getAllConnectionAttempts() {
    if (!this.networkAdapter) {
      return {};
    }
    return this.networkAdapter.getAllConnectionAttempts();
  }

  async toggleDiscovery(enabled: boolean) {
    if (!this.networkAdapter) {
      return;
    }
    return this.networkAdapter.toggleLocalDiscovery(enabled);
  }

  async restartDiscovery() {
    if (!this.networkAdapter) {
      return;
    }
    return this.networkAdapter.restartDiscovery();
  }

  async getP2PStatus() {
    if (!this.networkAdapter) {
      return null;
    }
    return this.networkAdapter.getP2PStatus();
  }

  async shutdown(): Promise<void> {
    if (this.networkAdapter) {
      this.networkAdapter.disconnect();
    }
    if (this.repo) {
      await this.repo.shutdown();
    }
    console.log('P2P sync shut down');
  }
}

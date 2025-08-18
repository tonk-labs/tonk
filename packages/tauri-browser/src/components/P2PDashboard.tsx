import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { P2PSync } from '../lib/p2p-sync';

interface PeerInfo {
  peer_id: string;
  bundle_id: string;
  connected: boolean;
}

interface TonkServiceInfo {
  bundle_id: string;
  node_id: string;
  port: number;
  addresses: string[];
}

export function P2PDashboard() {
  const [nodeId, setNodeId] = useState<string>('');
  const [bundleId, setBundleId] = useState<string>('test-bundle');
  const [p2pStatus, setP2PStatus] = useState<any>(null);
  const [connectedPeers, setConnectedPeers] = useState<PeerInfo[]>([]);
  const [discoveredPeers, setDiscoveredPeers] = useState<TonkServiceInfo[]>([]);
  const [, setP2PSync] = useState<P2PSync | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);

  useEffect(() => {
    refreshStatus();
    const interval = setInterval(refreshStatus, 2000);
    return () => clearInterval(interval);
  }, []);

  const refreshStatus = async () => {
    try {
      const [status, connected, discovered, node] = await Promise.all([
        invoke('get_p2p_status'),
        invoke('get_connected_peers'),
        invoke('get_discovered_peers'),
        invoke('get_node_id'),
      ]);

      setP2PStatus(status);
      setConnectedPeers(connected as PeerInfo[]);
      setDiscoveredPeers(discovered as TonkServiceInfo[]);
      setNodeId((node as string) || '');
    } catch (error) {
      console.error('Failed to refresh status:', error);
    }
  };

  const initializeP2P = async () => {
    try {
      const sync = new P2PSync();
      await sync.initialize(bundleId);
      setP2PSync(sync);
      setIsInitialized(true);
      await refreshStatus();
    } catch (error) {
      console.error('Failed to initialize P2P:', error);
    }
  };

  const connectToPeer = async (peerId: string) => {
    try {
      await invoke('connect_to_peer', { peerId });
      await refreshStatus();
    } catch (error) {
      console.error('Failed to connect to peer:', error);
    }
  };

  const restartDiscovery = async () => {
    try {
      await invoke('restart_discovery');
      await refreshStatus();
    } catch (error) {
      console.error('Failed to restart discovery:', error);
    }
  };

  return (
    <div className="dashboard">
      <h1>P2P Connection Tester</h1>

      <div className="status-section">
        <h2>Node Status</h2>
        <p>Node ID: {nodeId || 'Not initialized'}</p>
        <p>Bundle ID: {bundleId}</p>
        <p>Initialized: {isInitialized ? '✅' : '❌'}</p>

        <div className="controls">
          <input
            value={bundleId}
            onChange={e => setBundleId(e.target.value)}
            placeholder="Bundle ID"
            disabled={isInitialized}
          />
          <button onClick={initializeP2P} disabled={isInitialized}>
            Initialize P2P
          </button>
          <button onClick={restartDiscovery}>Restart Discovery</button>
          <button onClick={refreshStatus}>Refresh</button>
        </div>
      </div>

      <div className="peers-section">
        <h2>Connected Peers ({connectedPeers.length})</h2>
        {connectedPeers.length === 0 ? (
          <p>No connected peers</p>
        ) : (
          <ul>
            {connectedPeers.map(peer => (
              <li key={peer.peer_id}>
                <strong>{peer.peer_id.slice(0, 12)}...</strong> -{' '}
                {peer.bundle_id}
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="discovery-section">
        <h2>Discovered Peers ({discoveredPeers.length})</h2>
        {discoveredPeers.length === 0 ? (
          <p>No peers discovered</p>
        ) : (
          <ul>
            {discoveredPeers.map(peer => (
              <li key={peer.node_id}>
                <div>
                  <strong>{peer.node_id.slice(0, 12)}...</strong> -{' '}
                  {peer.bundle_id}
                  <br />
                  <small>
                    Port: {peer.port}, Addresses: {peer.addresses.length}
                  </small>
                </div>
                <button onClick={() => connectToPeer(peer.node_id)}>
                  Connect
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="debug-section">
        <h2>P2P Status</h2>
        <pre>{JSON.stringify(p2pStatus, null, 2)}</pre>
      </div>
    </div>
  );
}

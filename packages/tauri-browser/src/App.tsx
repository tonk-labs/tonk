import { useState, useEffect } from 'react';
import { P2PSync } from './lib/p2p-sync';
import { PeerInfo } from './lib/adapters/iroh';

function App() {
  const [p2pSync, setP2pSync] = useState<P2PSync | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [error, setError] = useState<string | null>(null);
  const bundleId = 'com.example.test-bundle';

  useEffect(() => {
    initializeP2P();
    return () => {
      if (p2pSync) {
        p2pSync.shutdown();
      }
    };
  }, []);

  const initializeP2P = async () => {
    try {
      const sync = new P2PSync();
      await sync.initialize(bundleId);
      setP2pSync(sync);
      setIsInitialized(true);

      // Start polling for peer updates
      const interval = setInterval(async () => {
        const connectedPeers = await sync.getConnectedPeers();
        setPeers(connectedPeers);
      }, 2000);

      return () => clearInterval(interval);
    } catch (err) {
      setError(`Failed to initialize P2P: ${err}`);
      console.error('P2P initialization error:', err);
    }
  };

  const refreshPeers = async () => {
    if (p2pSync) {
      try {
        const connectedPeers = await p2pSync.getConnectedPeers();
        setPeers(connectedPeers);
      } catch (err) {
        console.error('Failed to refresh peers:', err);
      }
    }
  };

  return (
    <div style={{ padding: '20px', fontFamily: 'system-ui, sans-serif' }}>
      <h1>🌐 Tonk P2P Browser</h1>

      <div style={{ marginBottom: '20px' }}>
        <h2>Status</h2>
        <p>P2P System: {isInitialized ? '✅ Ready' : '⏳ Initializing...'}</p>
        <p>Bundle ID: {bundleId}</p>
        {error && (
          <div
            style={{
              color: 'red',
              padding: '10px',
              border: '1px solid red',
              borderRadius: '4px',
            }}
          >
            Error: {error}
          </div>
        )}
      </div>

      <div style={{ marginBottom: '20px' }}>
        <h2>Connected Peers ({peers.length})</h2>
        <button
          onClick={refreshPeers}
          style={{ padding: '8px 16px', marginBottom: '10px' }}
          disabled={!isInitialized}
        >
          🔄 Refresh Peers
        </button>

        {peers.length === 0 ? (
          <p style={{ color: '#666' }}>
            No peers connected yet. Start another instance to test P2P sync!
          </p>
        ) : (
          <ul style={{ listStyle: 'none', padding: 0 }}>
            {peers.map(peer => (
              <li
                key={peer.peer_id}
                style={{
                  padding: '10px',
                  margin: '5px 0',
                  backgroundColor: '#f5f5f5',
                  borderRadius: '4px',
                  border: peer.connected ? '2px solid green' : '2px solid gray',
                }}
              >
                <div>
                  <strong>Peer ID:</strong> {peer.peer_id}
                </div>
                <div>
                  <strong>Bundle:</strong> {peer.bundle_id}
                </div>
                <div>
                  <strong>Status:</strong>{' '}
                  {peer.connected ? '🟢 Connected' : '🔴 Disconnected'}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div style={{ marginBottom: '20px' }}>
        <h2>Next Steps</h2>
        <ul>
          <li>Start another instance of this app to test P2P discovery</li>
          <li>
            The Iroh network adapter will automatically discover peers on the
            same network
          </li>
          <li>Once connected, Automerge documents will sync automatically</li>
        </ul>
      </div>

      <div style={{ fontSize: '12px', color: '#666', marginTop: '40px' }}>
        <p>This is a proof-of-concept P2P Tonk browser using:</p>
        <ul>
          <li>Tauri (Rust backend + Web frontend)</li>
          <li>Iroh (P2P networking)</li>
          <li>Automerge (CRDT data sync)</li>
        </ul>
      </div>
    </div>
  );
}

export default App;

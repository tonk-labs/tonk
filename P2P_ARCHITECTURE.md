# Tonk P2P Architecture Design

## Overview

This document outlines a peer-to-peer architecture for Tonk applications that eliminates the need
for centralized sync servers. Instead, devices running the same Tonk bundle will discover each other
and sync directly using P2P protocols.

## Current Architecture Analysis

### Existing Components

1. **Bundle Format** (`packages/spec`)
   - ZIP-based container with JSON manifest
   - Contains app files, metadata, and entrypoints
   - Version control and file integrity

2. **Browser Package** (`packages/browser`)
   - Electron app that loads bundles
   - Currently relies on local sync server (port 7777)
   - Uses Keepsync for data synchronization via WebSocket

3. **Keepsync Package** (`packages/keepsync`)
   - TypeScript implementation of Automerge-repo
   - Pluggable network adapter architecture
   - Already handles CRDT-based sync

4. **Server Package** (`packages/server`)
   - Local sync server for bundle serving
   - Automerge-based data synchronization
   - WebSocket connections for real-time sync

## Proposed P2P Architecture

### Core Technologies

- **Iroh**: P2P connectivity layer using QUIC (Rust)
  - Direct connections via public key dialing
  - NAT traversal and hole-punching
  - End-to-end encryption and authentication

- **Keepsync + Automerge**: Data synchronization (TypeScript)
  - Existing TypeScript implementation stays unchanged
  - Pluggable network adapter for Iroh integration
  - CRDT-based conflict resolution

- **Tauri**: Desktop application framework
  - Rust backend for Iroh integration
  - WebView frontend (existing React app)
  - IPC bridge between Rust and TypeScript

### Hybrid Architecture Design

```
┌────────────────────────────────────────────────┐
│            Tauri Application                   │
├────────────────────────────────────────────────┤
│                                                │
│  ┌──────────────────────────────────────────┐  │
│  │         Rust Backend (src-tauri)         │  │
│  │                                          │  │
│  │  • Iroh P2P Connectivity                 │  │
│  │  • Bundle Management                     │  │
│  │  • Network Transport Layer               │  │
│  │  • Tauri Command Handlers                │  │
│  └──────────────────────────────────────────┘  │
│                      ↕ IPC                     │
│  ┌──────────────────────────────────────────┐  │
│  │      TypeScript Frontend (src)           │  │
│  │                                          │  │
│  │  • React UI                              │  │
│  │  • Keepsync (Automerge-repo)             │  │
│  │  • IrohNetworkAdapter                    │  │
│  │  • Business Logic                        │  │
│  └──────────────────────────────────────────┘  │
│                                                │
└────────────────────────────────────────────────┘
```

### Key Integration: IrohNetworkAdapter

The bridge between Iroh (Rust) and Keepsync (TypeScript) is a custom NetworkAdapter:

```typescript
// packages/keepsync/src/adapters/iroh.ts
import { NetworkAdapter, Message, PeerId } from '@automerge/automerge-repo';
import { invoke, listen } from '@tauri-apps/api';

export class IrohNetworkAdapter extends NetworkAdapter {
  private peers: Map<PeerId, boolean> = new Map();

  constructor() {
    super();
    this.setupListeners();
  }

  private setupListeners() {
    // Listen for Automerge messages from Iroh connections
    listen('automerge_message', (event: any) => {
      const { peerId, message } = event.payload;
      // Forward to Automerge-repo for processing
      this.emit('message', {
        senderId: peerId as PeerId,
        targetId: this.peerId,
        type: message.type,
        data: message.data,
      });
    });

    // Listen for peer discovery events
    listen('peer_discovered', (event: any) => {
      const { peerId, bundleId } = event.payload;
      this.peers.set(peerId, true);
      // Auto-connect to discovered peers
      this.connect(peerId);
    });

    // Listen for connection status
    listen('peer_connected', (event: any) => {
      this.emit('ready', { network: this });
    });
  }

  send(message: Message) {
    // Send Automerge sync messages through Iroh
    invoke('send_automerge_message', {
      targetId: message.targetId,
      message: {
        type: message.type,
        data: Array.from(message.data), // Convert Uint8Array for IPC
      },
    });
  }

  connect(peerId: PeerId) {
    return invoke('connect_to_peer', { peerId });
  }

  disconnect() {
    this.peers.clear();
    invoke('disconnect_all_peers');
  }
}
```

### Rust Backend Commands

```rust
// src-tauri/src/commands.rs
use tauri::{State, Manager};
use crate::p2p::P2PManager;

#[tauri::command]
async fn connect_to_peer(
    peer_id: String,
    p2p: State<'_, P2PManager>,
) -> Result<(), String> {
    p2p.connect_peer(&peer_id).await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn send_automerge_message(
    target_id: String,
    message: AutomergeMessage,
    p2p: State<'_, P2PManager>,
) -> Result<(), String> {
    // Get connection for target peer
    let conn = p2p.get_connection(&target_id)
        .ok_or("Peer not connected")?;

    // Serialize and send raw Automerge sync message
    let bytes = serde_json::to_vec(&message)
        .map_err(|e| e.to_string())?;

    conn.send(bytes).await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_discovery(
    bundle_id: String,
    p2p: State<'_, P2PManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let app_clone = app.clone();

    p2p.start_discovery(&bundle_id, move |peer_info| {
        // Emit peer discovery event to frontend
        app_clone.emit_all("peer_discovered", peer_info).ok();
    }).await.map_err(|e| e.to_string())
}
```

### Bundle Metadata Extensions

```typescript
interface P2PBundleManifest extends BundleManifest {
  // P2P Discovery Metadata
  sync: {
    // Unique bundle identifier (content-addressed hash)
    bundleId: string; // SHA-256 of bundle content

    // App-specific namespace for discovery
    namespace: string; // e.g., "com.example.myworld"

    // Version info for compatibility
    protocolVersion: string;
    minCompatibleVersion?: string;

    // Discovery hints
    discovery: {
      // DHT key for finding peers
      dhtKey?: string;

      // mDNS service name for local discovery
      mdnsService?: string;

      // Known bootstrap peers (optional)
      bootstrapPeers?: string[];
    };
  };
}
```

### P2P Manager (Rust)

```rust
// src-tauri/src/p2p/mod.rs
use iroh::node::Node;
use iroh::net::{NodeId, NodeAddr};
use iroh_gossip::net::Gossip;
use std::collections::HashMap;
use tokio::sync::RwLock;

pub struct P2PManager {
    node: Node,
    gossip: Gossip,
    connections: Arc<RwLock<HashMap<String, Connection>>>,
    bundle_id: Option<String>,
}

impl P2PManager {
    pub async fn new() -> Result<Self> {
        let node = Node::memory().spawn().await?;
        let gossip = Gossip::from_endpoint(
            node.endpoint().clone(),
            Default::default(),
        );

        Ok(Self {
            node,
            gossip,
            connections: Arc::new(RwLock::new(HashMap::new())),
            bundle_id: None,
        })
    }

    pub async fn load_bundle(&mut self, bundle_id: String) -> Result<()> {
        // Join gossip topic for bundle
        let topic = TopicId::from_bytes(bundle_id.as_bytes());
        self.gossip.join(topic, vec![]).await?;

        // Start mDNS discovery
        self.start_mdns_discovery(&bundle_id).await?;

        self.bundle_id = Some(bundle_id);
        Ok(())
    }

    pub async fn connect_peer(&self, peer_id: &str) -> Result<()> {
        let node_id = NodeId::from_str(peer_id)?;
        let conn = self.node.connect(node_id, ALPN_TONK_SYNC).await?;

        // Store connection
        let mut connections = self.connections.write().await;
        connections.insert(peer_id.to_string(), conn);

        // Start receiving messages from this peer
        self.start_message_receiver(peer_id.to_string(), conn.clone()).await;

        Ok(())
    }

    async fn start_message_receiver(&self, peer_id: String, mut conn: Connection) {
        let app = self.app_handle.clone();

        tokio::spawn(async move {
            while let Ok(message) = conn.recv().await {
                // Parse as Automerge message and forward to frontend
                if let Ok(automerge_msg) = serde_json::from_slice(&message) {
                    app.emit_all("automerge_message", json!({
                        "peerId": peer_id,
                        "message": automerge_msg
                    })).ok();
                }
            }
        });
    }
}
```

### Frontend Integration

```typescript
// packages/tonk-browser-rust/src/lib/p2p-sync.ts
import { SyncEngine } from '@tonk/keepsync';
import { IrohNetworkAdapter } from '@tonk/keepsync/adapters/iroh';
import { IndexedDBStorageAdapter } from '@automerge/automerge-repo-storage-indexeddb';
import { invoke } from '@tauri-apps/api/tauri';

export class P2PSync {
  private syncEngine: SyncEngine;
  private bundleId: string | null = null;

  async initialize(bundleId: string) {
    this.bundleId = bundleId;

    // Start P2P discovery in Rust backend
    await invoke('start_discovery', { bundleId });

    // Initialize Keepsync with Iroh network adapter
    this.syncEngine = new SyncEngine({
      url: '', // No central server
      network: [new IrohNetworkAdapter()],
      storage: new IndexedDBStorageAdapter('tonk-p2p'),
      peerId: (await invoke('get_node_id')) as PeerId,
    });

    await this.syncEngine.whenReady();
  }

  // Use existing Keepsync APIs for data operations
  async writeDoc(path: string, data: any) {
    return this.syncEngine.writeDoc(path, data);
  }

  async readDoc(path: string) {
    return this.syncEngine.readDoc(path);
  }

  async ls(path: string) {
    return this.syncEngine.ls(path);
  }
}
```

### Implementation Plan

#### Phase 1: Core Infrastructure

1. Set up Tauri project structure
2. Integrate Iroh in Rust backend
3. Create IrohNetworkAdapter for Keepsync
4. Implement basic IPC commands

#### Phase 2: P2P Discovery

1. Implement mDNS discovery
2. Add DHT-based discovery
3. Handle peer connection lifecycle
4. Test local network discovery

#### Phase 3: Data Sync Integration

1. Connect IrohNetworkAdapter to Keepsync
2. Test Automerge message passing
3. Verify CRDT sync works over P2P
4. Handle connection failures/reconnects

#### Phase 4: Bundle Management

1. Port bundle loading to Rust
2. Add P2P metadata to bundle format
3. Implement bundle verification
4. Test with example apps

#### Phase 5: UI and Polish

1. Add peer status indicators
2. Show sync progress
3. Handle offline/online transitions
4. Performance optimization

### Benefits of Hybrid Approach

1. **Reuse Existing Code**: Keep all Keepsync/Automerge TypeScript code
2. **Minimal Changes**: Only need to add network adapter
3. **Best of Both Worlds**: Iroh's P2P + Keepsync's proven sync
4. **Gradual Migration**: Can migrate more to Rust over time
5. **Faster Development**: 4-6 weeks vs 10-12 weeks for full rewrite

### Performance Expectations

| Component         | Implementation        | Performance Impact |
| ----------------- | --------------------- | ------------------ |
| P2P Discovery     | Rust (Iroh)           | Fast, native       |
| Network Transport | Rust (Iroh)           | High throughput    |
| Data Sync Logic   | TypeScript (Keepsync) | Same as current    |
| UI Rendering      | React                 | Same as current    |
| IPC Overhead      | Tauri                 | Minimal (~1ms)     |

### Migration Path

```typescript
// Step 1: Add P2P mode to existing Keepsync
const syncEngine = new SyncEngine({
  url: useP2P ? '' : 'http://localhost:7777',
  network: useP2P ? [new IrohNetworkAdapter()] : [new WebSocketAdapter()],
  storage: new IndexedDBStorageAdapter('tonk'),
});

// Step 2: Gradual adoption
if (window.__TAURI__) {
  // Running in Tauri with P2P
  initP2PSync();
} else {
  // Fallback to server-based sync
  initServerSync();
}
```

### Security Considerations

1. **Bundle Verification**
   - Content-addressed bundle IDs (SHA-256)
   - Verify bundle integrity before loading

2. **Peer Authentication**
   - Iroh's Ed25519 public key identity
   - Automatic via QUIC handshake

3. **Message Validation**
   - Validate Automerge messages before processing
   - Rate limiting on message handling

4. **Data Privacy**
   - End-to-end encryption via Iroh
   - No data on third-party servers

## Next Steps

1. Create proof-of-concept Tauri app with Iroh
2. Implement IrohNetworkAdapter for Keepsync
3. Test P2P sync between two instances
4. Benchmark performance vs server approach
5. Package example app (my-world) with P2P

## Resources

- [Iroh Documentation](https://iroh.computer/docs)
- [Tauri Guides](https://tauri.app/v1/guides/)
- [Keepsync Source](packages/keepsync)
- [Automerge Repo](https://github.com/automerge/automerge-repo)

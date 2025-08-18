# Tonk P2P Browser

A peer-to-peer Tonk browser built with Tauri, Iroh, and Automerge for decentralized data
synchronization.

## Architecture

This is a hybrid application that combines:

- **Rust Backend (src-tauri/)**: Iroh P2P networking, discovery, and transport layer
- **TypeScript Frontend (src/)**: React UI with Keepsync/Automerge for data sync
- **IPC Bridge**: Tauri commands connecting Rust P2P with TypeScript CRDT sync

## Key Components

### Rust Backend

- `P2PManager`: Core P2P functionality using Iroh
- `IrohNetworkAdapter`: Bridge between Iroh and Automerge
- Tauri commands for IPC communication

### TypeScript Frontend

- `IrohNetworkAdapter`: Keepsync network adapter for P2P
- `P2PSync`: High-level P2P sync manager
- React UI for peer status and discovery

## Development

### Prerequisites

- Rust (latest stable)
- Node.js 18+ with pnpm
- Tauri CLI: `cargo install tauri-cli`

### Setup

```bash
cd packages/tauri-browser

# Install dependencies
pnpm install

# Build TypeScript
pnpm run build

# Run in development mode
pnpm run tauri:dev

# Or build for production
pnpm run tauri:build
```

### Development Commands

```bash
# Frontend development (with hot reload)
pnpm run dev

# Build frontend only
pnpm run build

# Run Tauri app
pnpm run tauri:dev

# Check Rust code
cd src-tauri && cargo check
```

## Phase 1 Implementation Status ✅

**Completed Features:**

- ✅ Tauri project setup with Rust + TypeScript
- ✅ Iroh P2P integration in Rust backend
- ✅ Basic IPC commands (connect_to_peer, send_automerge_message, start_discovery)
- ✅ IrohNetworkAdapter for Keepsync integration
- ✅ React UI with peer status display
- ✅ WASM build support for Automerge

**Next Phases:**

- **Phase 2**: Real P2P discovery (mDNS, DHT)
- **Phase 3**: Complete CRDT sync over P2P
- **Phase 4**: Bundle management and serving
- **Phase 5**: UI polish and testing

## Testing

Currently the app displays a basic UI showing P2P system status. To test:

1. Start the app: `pnpm run tauri:dev`
2. The UI will show "P2P System: ✅ Ready" when initialized
3. Multiple instances can be started for peer discovery testing (Phase 2)

## Architecture Benefits

- **Reuses existing Keepsync/Automerge code** (90% unchanged)
- **Native P2P performance** via Rust/Iroh
- **Gradual migration path** from server-based to P2P
- **Cross-platform** desktop support via Tauri

## File Structure

```
packages/tauri-browser/
├── src-tauri/           # Rust backend
│   ├── src/
│   │   ├── main.rs      # Entry point
│   │   ├── lib.rs       # Tauri app setup
│   │   ├── commands.rs  # IPC commands
│   │   └── p2p/         # P2P modules
│   │       ├── manager.rs   # Core P2P manager
│   │       ├── discovery.rs # Peer discovery
│   │       └── sync.rs      # Sync protocols
│   ├── Cargo.toml       # Rust dependencies
│   └── tauri.conf.json  # Tauri configuration
├── src/                 # TypeScript frontend
│   ├── App.tsx          # Main React component
│   ├── main.tsx         # React entry point
│   └── lib/
│       ├── p2p-sync.ts  # High-level P2P sync
│       └── adapters/
│           └── iroh.ts  # Keepsync network adapter
├── package.json         # Node.js dependencies
├── vite.config.ts       # Build configuration
└── README.md
```

---

This represents the foundation for a fully peer-to-peer Tonk browser that eliminates the need for
centralized servers while maintaining compatibility with existing Tonk applications.

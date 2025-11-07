# Tonk Relay

This is the **free, open-source relay server** for Tonk. Perfect for development, testing, and
non-demanding use cases.

## Features

- **WebSocket Sync Server**: Full automerge-repo protocol support using samod
- **Bundle Management**: Serve tonk bundles with manifest support
- **S3 Integration**: Optional bundle storage in AWS S3
- **Wire Compatible**: Works with existing TypeScript clients without changes
- **High Performance**: Leverages Rust's async runtime for better performance and lower memory usage

## Building

```bash
cargo build --release
```

## Running

```bash
# Basic usage
./target/release/tonk-relay <port> <bundle-path> [storage-dir]

# Example
./target/release/tonk-relay 8081 latergram.tonk ./relay-storage
```

### Arguments

1. **port** (required): HTTP and WebSocket server port
2. **bundle-path** (required): Path to the .tonk bundle file
3. **storage-dir** (optional): Directory for automerge storage (default: `automerge-repo-data`)

### Environment Variables

Copy `.env.example` to `.env` and configure:

- `S3_BUCKET_NAME`: AWS S3 bucket for bundle storage (optional)
- `AWS_REGION`: AWS region (default: `eu-north-1`)
- `RUST_LOG`: Log level (`error`, `warn`, `info`, `debug`, `trace`)

## Architecture

- **HTTP Server** (port): Serves API endpoints, bundle manifests, and static files
- **WebSocket Server** (port): Handles automerge sync connections
- **Storage**:
  - Filesystem storage for automerge documents (compatible with automerge-repo-storage-nodefs)
  - Bundle storage for serving tonk bundles
  - Optional S3 storage for bundle uploads/downloads

## API Endpoints

- `GET /` - Health check
- `GET /tonk_core_bg.wasm` - Serve WASM file
- `GET /.manifest.tonk` - Get slim bundle (manifest + root doc)
- `GET /metrics` - Server metrics (connections, memory, uptime)
- `POST /api/bundles` - Upload bundle to S3 (requires S3 config)
- `GET /api/bundles/:id` - Download full bundle from S3
- `GET /api/bundles/:id/manifest` - Download slim bundle from S3
- `GET /api/blank-tonk` - Download blank tonk template

## Wire Compatibility

This Rust implementation is fully wire-compatible with:

- TypeScript automerge-repo clients
- samod
- tonk-core

Storage files created by the TypeScript version can be read by the Rust version and vice versa.

## Development

```bash
# Run with debug logging
RUST_LOG=debug cargo run -- 8081 latergram.tonk ./relay-storage

# Check for compilation errors
cargo check

# Run tests
cargo test

# Auto-rebuild on changes
cargo watch -x run
```

### Internal Development

Internal developers at Tonk Labs can use the proprietary relay for development:

```bash
# Use proprietary relay from knot repo
nix develop .#withKnot --override-input knot path:../knot
```

This sets the `TONK_RELAY_BINARY` environment variable to point to the proprietary relay, and all
tools (tests, mprocs, etc.) will automatically use it.

## License

Released under the MIT License. See [LICENSE](../../LICENSE) for details.

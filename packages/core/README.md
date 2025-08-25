# Tonk Core WASM Bindings

This package provides WebAssembly (WASM) bindings for Tonk Core, enabling you to use Tonk's
local-first sync engine in JavaScript environments (browsers and Node.js).

## Features

- **Sync Engine**: Create and manage synchronization engines with peer-to-peer capabilities
- **Virtual File System (VFS)**: Create, read, update, and delete files and directories
- **Bundle Operations**: Work with Tonk bundle format for efficient data storage
- **WebSocket Support**: Connect to other peers for real-time synchronization
- **Cross-platform**: Works in browsers and Node.js

## Installation

### Prerequisites

- Rust toolchain (1.70 or later)
- wasm-pack (`curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh`)
- Node.js (14 or later) for Node.js usage

### Building from Source

```bash
# Clone the repository
git clone https://github.com/tonk-labs/tonk.git
cd tonk/packages/core

# Build for web browsers
wasm-pack build --target web --out-dir pkg

# Build for Node.js
wasm-pack build --target nodejs --out-dir pkg-node

# Build for bundlers (webpack, rollup, etc.)
wasm-pack build --target bundler --out-dir pkg-bundler

# Or use the convenience script
./build-wasm.sh
```

## Usage

### Browser Example

```javascript
import init, { create_sync_engine } from './pkg/tonk_core.js';

async function main() {
  // Initialize the WASM module
  await init();

  // Create a sync engine
  const engine = await create_sync_engine();
  const peerId = await engine.getPeerId();
  console.log('Peer ID:', peerId);

  // Get VFS instance
  const vfs = await engine.getVfs();

  // Create a file
  await vfs.createFile('/hello.txt', 'Hello, World!');

  // List directory
  const files = await vfs.listDirectory('/');
  console.log('Files:', files);
}

main().catch(console.error);
```

### Node.js Example

```javascript
const { create_sync_engine } = require('./pkg-node/tonk_core.js');

async function main() {
  const engine = await create_sync_engine();
  const vfs = await engine.getVfs();

  // Create files and directories
  await vfs.createDirectory('/documents');
  await vfs.createFile('/documents/note.txt', 'My first note');

  // Check if file exists
  const exists = await vfs.exists('/documents/note.txt');
  console.log('File exists:', exists);
}

main().catch(console.error);
```

## API Reference

### SyncEngine

#### `create_sync_engine()`

Creates a new sync engine with a randomly generated peer ID.

#### `create_sync_engine_with_peer_id(peerId: string)`

Creates a new sync engine with a specific peer ID.

#### `engine.getPeerId()`

Returns the peer ID of the sync engine.

#### `engine.connectWebsocket(url: string)`

Connects to a WebSocket server for peer synchronization.

#### `engine.getVfs()`

Returns the VFS instance associated with the engine.

### Virtual File System (VFS)

#### `vfs.createFile(path: string, content: string)`

Creates a new file with the specified content.

#### `vfs.readFile(path: string)`

Reads the content of a file (returns null if not found).

#### `vfs.deleteFile(path: string)`

Deletes a file.

#### `vfs.createDirectory(path: string)`

Creates a new directory.

#### `vfs.listDirectory(path: string)`

Lists the contents of a directory.

#### `vfs.exists(path: string)`

Checks if a file or directory exists.

#### `vfs.getMetadata(path: string)`

Gets metadata about a file or directory.

### Bundle Operations

#### `create_bundle_from_bytes(data: Uint8Array)`

Creates a bundle from byte array data.

#### `bundle.get(key: string)`

Gets a value from the bundle by key.

#### `bundle.put(key: string, value: Uint8Array)`

Stores a value in the bundle.

#### `bundle.delete(key: string)`

Deletes a value from the bundle.

#### `bundle.listKeys()`

Returns all keys in the bundle.

#### `bundle.getPrefix(prefix: string)`

Returns all key-value pairs matching the prefix.

## Examples

See the `examples/` directory for complete examples:

- `index.html` - Interactive browser demo
- `node-example.js` - Node.js usage example

## Development

### Running Tests

```bash
# Run WASM tests
wasm-pack test --headless --firefox

# Run Rust tests
cargo test
```

### Building Documentation

```bash
cargo doc --no-deps --open
```

## Performance Considerations

- The WASM module uses `wee_alloc` for smaller binary size (optional feature)
- All async operations return JavaScript Promises
- Large file operations should be batched when possible
- WebSocket connections are managed efficiently with automatic reconnection

## License

MIT License - see LICENSE file for details

## Contributing

Contributions are welcome! Please see the main repository's CONTRIBUTING.md for guidelines.

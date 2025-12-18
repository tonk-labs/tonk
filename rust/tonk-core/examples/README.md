# Tonk Core Examples

This directory contains examples and integration tests for Tonk Core across different platforms and
environments.

## Directory Structure

```
examples/
├── node/                 # Node.js integration tests and examples
│   ├── integration/      # Comprehensive test suite
│   ├── examples/         # Standalone example scripts
│   └── package.json      # Node-specific dependencies
├── browser/              # Browser examples (future)
│   └── index.html        # Basic browser example
├── shared/               # Shared utilities across platforms
│   ├── test-utils.js     # Common testing helpers
│   └── test-server.js    # Mock WebSocket server
└── README.md            # This file
```

## Quick Start

### Node.js Examples

```bash
# Build WASM for Node.js
npm run build:node

# Go to Node examples
cd examples/node

# Install dependencies
npm install

# Run basic example
npm run example:basic

# Run integration tests
npm test
```

### Browser Examples (Coming Soon)

```bash
# Build WASM for browsers
npm run build

# Serve browser examples
cd examples/browser
# Open index.html in browser
```

## What's Included

### Node.js Integration (`node/`)

**Integration Tests:**

- ✅ **Basic Tests**: Sync engines, VFS operations, bundle management
- ✅ **Bundle Tests**: File I/O, serialization, large data handling
- ✅ **Sync Tests**: Multi-engine scenarios, persistence, performance
- ✅ **WebSocket Tests**: Network infrastructure, future sync capabilities

**Examples:**

- ✅ **Basic Usage**: Complete API walkthrough with error handling
- ✅ **File Sync**: Multi-engine synchronization demonstration
- ✅ **Collaborative Edit**: Multi-user editing simulation

**Test Infrastructure:**

- ✅ **Test Utilities**: Data generators, performance tools, WASM loading
- ✅ **Mock Server**: WebSocket server for integration testing
- ✅ **Comprehensive Coverage**: 100+ test cases across all features

### Browser Integration (`browser/`) - Future

**Planned Features:**

- 🚧 **Basic HTML Example**: Simple browser integration
- 🚧 **Web Worker Support**: Background processing
- 🚧 **IndexedDB Integration**: Browser persistence
- 🚧 **WebRTC Sync**: Peer-to-peer synchronization
- 🚧 **Service Worker**: Offline capabilities

### Shared Utilities (`shared/`)

**Available Now:**

- ✅ **Test Data Generators**: Realistic test scenarios
- ✅ **Performance Tools**: Benchmarking and profiling
- ✅ **Mock WebSocket Server**: Network testing infrastructure
- ✅ **Common Helpers**: File handling, async utilities

## Current Capabilities

### ✅ Working Features

**Sync Engine:**

- Create engines with random or custom peer IDs
- Multiple engines in single process
- Engine lifecycle management
- Performance: < 100ms creation time

**Virtual File System:**

- Create/delete files and directories
- Hierarchical path support
- Metadata and directory listing
- Performance: > 1000 ops/sec

**Bundle Operations:**

- Create and manage bundles
- Store/retrieve arbitrary data
- Serialization and deserialization
- File I/O integration
- Performance: > 500 bundles/sec

**Testing Infrastructure:**

- Comprehensive test coverage
- Performance benchmarking
- Mock server for network testing
- Cross-platform utilities

### 🚧 In Development

**WebSocket Synchronization:**

- Real-time peer communication
- Change propagation
- Connection management

**CRDT Operations:**

- Conflict-free merging
- Distributed editing
- Consistency guarantees

### 🔮 Future Capabilities

**Advanced Sync:**

- Multi-peer networks
- Peer discovery
- Offline sync
- Mobile support

**Collaboration Features:**

- Live cursors
- Presence indicators
- Real-time collaboration
- Version control integration

## Performance Benchmarks

Current performance characteristics:

| Operation            | Rate               | Notes              |
| -------------------- | ------------------ | ------------------ |
| Engine Creation      | ~10-50 engines/sec | Varies by system   |
| File Creation        | > 1000 files/sec   | Small files        |
| Directory Listing    | > 2000 lists/sec   | Cached operations  |
| Bundle Creation      | > 500 bundles/sec  | Empty bundles      |
| Bundle Serialization | ~100MB/sec         | Depends on content |
| Large File Storage   | ~30MB/sec          | 100KB+ files       |

## Getting Started Guide

### 1. Environment Setup

```bash
# Ensure you have Rust and wasm-pack
rustup target add wasm32-unknown-unknown
cargo install wasm-pack

# Clone and setup project
git clone <repository>
cd tonk/packages/core
```

### 2. Build WASM Module

```bash
# For Node.js
npm run build:node

# For browsers
npm run build

# For all targets
npm run build:all
```

### 3. Run Examples

```bash
# Start with basic Node.js example
cd examples/node
npm install
npm run example:basic
```

### 4. Explore Integration Tests

```bash
# Run comprehensive test suite
npm test

# Watch mode for development
npm run test:watch

# Verbose output with benchmarks
npm run test:verbose
```

## Development Workflow

### Adding New Features

1. **Write Tests First**: Add to `node/integration/`
2. **Implement WASM Bindings**: Update `src/wasm.rs`
3. **Add Examples**: Create in `node/examples/`
4. **Update Documentation**: Keep READMEs current
5. **Performance Test**: Include benchmarks

### Testing Strategy

1. **Unit Tests**: Test individual components
2. **Integration Tests**: Test WASM bindings end-to-end
3. **Performance Tests**: Ensure acceptable performance
4. **Error Tests**: Verify error handling
5. **Cross-Platform**: Test Node.js and browser environments

## Architecture Overview

```
┌─────────────────┐
│   Examples      │  ← User-facing demos and tests
├─────────────────┤
│  WASM Bindings  │  ← wasm-bindgen generated code
├─────────────────┤
│   Tonk Core     │  ← Rust implementation
├─────────────────┤
│     Samod       │  ← CRDT sync engine
├─────────────────┤
│   Automerge     │  ← CRDT implementation
└─────────────────┘
```

## Contributing

### Pull Request Guidelines

1. ✅ All tests must pass
2. ✅ Add tests for new functionality
3. ✅ Update documentation
4. ✅ Include performance considerations
5. ✅ Follow existing code patterns

### Issue Reporting

When reporting issues:

1. Include platform and Node.js version
2. Provide minimal reproduction case
3. Include error messages and stack traces
4. Note performance impacts if relevant

## Troubleshooting

### Build Issues

```bash
# Clean and rebuild
npm run clean
npm run build:all
```

### Test Failures

```bash
# Run specific test file
npx mocha integration/basic.test.js

# Debug with increased timeout
npm test -- --timeout 30000
```

### Performance Issues

```bash
# Profile with built-in benchmarks
npm run test:verbose

# Enable debug logging
DEBUG=tonk:* npm test
```

## Resources

- **[Node.js Examples](./node/README.md)**: Detailed Node.js documentation
- **[Tonk Core Docs](../README.md)**: Core library documentation
- **[Samod Docs](../../samod/README.md)**: Sync engine documentation
- **[Automerge Docs](https://automerge.org/)**: CRDT library documentation

---

**Status**: Node.js examples and tests are complete and ready for use. Browser examples coming soon!

# Tonk Core Node.js Examples and Integration Tests

This directory contains comprehensive Node.js integration tests and examples for Tonk Core WASM
bindings.

## Structure

```
node/
├── package.json          # Node dependencies and scripts
├── integration/          # Integration test suite
│   ├── basic.test.js     # Basic VFS and engine tests
│   ├── bundle.test.js    # Bundle operations tests
│   ├── sync.test.js      # Sync engine tests
│   └── websocket.test.js # WebSocket integration tests
├── examples/             # Standalone example scripts
│   ├── basic-usage.js    # Basic API usage
│   ├── file-sync.js      # File synchronization demo
│   └── collaborative-edit.js # Collaborative editing demo
└── README.md            # This file
```

## Quick Start

### Prerequisites

1. Build the WASM module:

   ```bash
   cd ../..  # Go to tonk-core root
   npm run build:node
   ```

2. Install Node dependencies:
   ```bash
   cd examples/node
   npm install
   ```

### Running Examples

```bash
# Basic usage example
npm run example:basic

# File synchronization demo
npm run example:sync

# Collaborative editing demo
npm run example:collab
```

### Running Integration Tests

```bash
# Run all tests
npm test

# Run tests with verbose output
npm run test:verbose

# Run tests in watch mode (for development)
npm run test:watch
```

## Integration Tests

### basic.test.js

- Sync engine creation and management
- VFS operations (create, read, list, delete)
- Bundle operations (create, store, retrieve)
- Error handling and edge cases
- Performance benchmarks

### bundle.test.js

- Bundle creation and serialization
- File I/O with bundles
- Large file handling
- Hierarchical path management
- Data integrity verification

### sync.test.js

- Engine lifecycle management
- Multi-engine scenarios
- VFS persistence across instances
- Memory management
- Performance profiling

### websocket.test.js

- WebSocket server infrastructure
- Connection handling (currently stubs)
- Future sync functionality tests
- Error handling for network operations

## Examples

### basic-usage.js

A comprehensive introduction to Tonk Core covering:

- Creating sync engines
- Basic VFS operations
- Bundle management
- Performance demonstration

### file-sync.js

Multi-engine file synchronization demo showing:

- Multiple sync engines
- Bundle-based persistence
- Conflict scenarios
- Performance testing
- WebSocket connection attempts

### collaborative-edit.js

Collaborative editing simulation featuring:

- Multi-user document editing
- Change tracking and history
- Conflict resolution scenarios
- Version management
- Real-time sync concepts

## Test Utilities

The `shared/` directory contains reusable utilities:

- `test-utils.js`: Common testing helpers, data generators, performance tools
- `test-server.js`: Mock WebSocket server for integration testing

## Key Features Demonstrated

### ✅ Currently Working

- Sync engine creation with custom/random peer IDs
- VFS operations (files, directories, metadata)
- Bundle operations (create, store, serialize, load)
- Multi-engine scenarios
- Performance benchmarking
- Error handling and recovery

### 🚧 In Development

- WebSocket-based synchronization
- Real-time collaboration
- CRDT-based conflict resolution
- Peer discovery and connection

### 🔮 Future Capabilities

- Live cursor tracking
- Presence indicators
- Offline synchronization
- Automatic conflict resolution
- Mobile platform support

## Performance Expectations

Based on current tests, you can expect:

- **Engine Creation**: < 100ms average
- **File Operations**: > 1000 ops/sec
- **Bundle Operations**: > 500 bundles/sec
- **Large Files**: 100KB files in < 3sec
- **Concurrent Operations**: 50+ parallel ops

## Troubleshooting

### Common Issues

**"Failed to load WASM module"**

```bash
# Build the WASM module first
cd ../..
npm run build:node
```

**"Module not found" errors**

```bash
# Install dependencies
npm install
```

**Tests timeout**

```bash
# Increase timeout or check system resources
npm test -- --timeout 10000
```

### Debug Mode

Enable detailed logging:

```bash
DEBUG=tonk:* npm test
```

## Development

### Adding New Tests

1. Create test file in `integration/`
2. Follow existing patterns with `describe/it` blocks
3. Use shared utilities from `../../shared/test-utils.js`
4. Include both positive and negative test cases
5. Add performance benchmarks where relevant

### Adding New Examples

1. Create example file in `examples/`
2. Add npm script in `package.json`
3. Include comprehensive error handling
4. Document the example's purpose and features
5. Update this README

### Test Infrastructure

The test suite uses:

- **Mocha**: Test framework
- **Chai**: Assertion library
- **Sinon**: Mocking and spies
- **WebSocket**: For server testing
- **Custom utilities**: WASM loading, data generation

## Contributing

1. Ensure all tests pass: `npm test`
2. Add tests for new functionality
3. Update documentation
4. Follow existing code style
5. Include performance considerations

## Related Documentation

- [Tonk Core README](../../README.md)
- [WASM Build Instructions](../../package.json)
- [Browser Examples](../browser/README.md)
- [Samod Documentation](../../../samod/README.md)

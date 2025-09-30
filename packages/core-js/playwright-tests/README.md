# Playwright Test Suite for Tonk Core JS

## Overview

This is a comprehensive benchmarking and stress testing suite for Tonk Core JS, designed to test
performance with high throughput scenarios and large-scale image handling (100s to 1000s of images).

## Setup

1. **Install dependencies:**

```bash
cd playwright-tests
npm install
```

2. **Install Playwright browsers:**

```bash
npx playwright install chromium
```

## Architecture

### Core Components

- **Server Manager**: Spawns unique Tonk server instances for each test
- **Image Generator**: Creates synthetic images for stress testing
- **Metrics Collector**: Tracks performance metrics in real-time
- **VFS Service**: WebWorker-based virtual file system operations
- **Test UI**: React application for Playwright interaction

## Running Tests

### Run all tests

```bash
npm test
```

### Run benchmarks only

```bash
npm run test:benchmark
```

### Run stress tests only

```bash
npm run test:stress
```

### Run tests with UI (for debugging)

```bash
npm run test:ui
```

### Run tests in headed mode (see browser)

```bash
npm run test:headed
```

## Test Categories

### Benchmarks (`tests/benchmarks/`)

- **throughput.spec.ts**: Sequential and parallel operation performance
- **images.spec.ts**: Image upload and processing performance
- **concurrent.spec.ts**: Multi-client concurrent operations
- **memory.spec.ts**: Memory usage and leak detection

### Stress Tests (`tests/stress/`)

- **large-scale.spec.ts**: 1000+ file operations
- **sync-storm.spec.ts**: Rapid synchronization under stress
- **network-failure.spec.ts**: Network resilience and recovery

## Performance Metrics

Each test collects:

- **Throughput**: Operations/second, MB/second
- **Latency**: Min, mean, p95, p99, max
- **Memory**: Heap usage, WASM memory, IndexedDB size
- **Errors**: Count and types

## Configuration

Tests use isolated server instances (ports 8100-8999) and the same `blank.tonk` bundle for
consistency.

### Key Settings

- Test timeout: 5 minutes
- Server startup timeout: 30 seconds
- Request timeout: 30 seconds
- Memory leak threshold: 50MB

## Development

### Start the test UI dev server

```bash
npm run dev
```

### View test reports

```bash
npm run report
```

## Test Data

### Synthetic Images

- Dimensions: iPhone photo sizes (3024x4032, etc.)
- Sizes: 1-10MB per image
- Formats: JPEG, PNG, WebP
- Metadata: EXIF-like data

### Performance Targets (Initial)

- Sequential ops: >100 ops/sec
- Parallel ops: >500 ops/sec
- Image uploads: >10 MB/sec
- Memory: <500MB for 1000 files

## Troubleshooting

### Server not starting

- Check if ports 8100-8999 are available
- Verify `blank.tonk` file exists at the expected path
- Check server logs in console output

### Memory issues

- Increase Node.js heap size: `NODE_OPTIONS="--max-old-space-size=4096"`
- Enable precise memory info in Chrome flags
- Check for memory leaks with the memory test suite

### Test failures

- Check test-results.json for detailed error information
- Use `npm run test:debug` for step-by-step debugging
- Review video recordings in test-results/ folder

## Implementation Status

✅ Core infrastructure

- Project structure
- Server manager
- Image generator
- Metrics collector
- Tonk worker
- VFS service

🚧 In Progress

- Test UI components
- Playwright configuration

⏳ Pending

- Throughput benchmarks
- Image stress tests
- Concurrent operations tests
- Memory monitoring tests
- Network failure tests

## Notes

- Tests run sequentially to avoid port conflicts
- Each test gets its own server instance
- Servers are automatically cleaned up after tests
- Memory snapshots are taken periodically for leak detection

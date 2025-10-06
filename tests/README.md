# Relay Playwright Tests

Comprehensive playwright test suite for the Tonk relay server testing websocket sync, bundle
operations, and IndexedDB storage.

## Setup

```bash
cd /Users/jackdouglas/tonk/tonk/packages/relay/playwright-tests
pnpm install
```

## Running Tests

```bash
# Run all tests
pnpm test

# Run specific test suites
pnpm test:sync      # WebSocket sync tests
pnpm test:bundles   # Bundle operations tests
pnpm test:stress    # Stress tests

# Run with UI
pnpm test:ui

# Run in headed mode
pnpm test:headed

# Debug mode
pnpm test:debug
```

## Test Structure

- `tests/sync/` - WebSocket synchronization tests between multiple clients
- `tests/bundles/` - Bundle upload, download, and sharing tests
- `tests/stress/` - Stress and performance tests

## Key Features

- **Isolated Test Servers**: Each test spins up its own relay server on a random port
- **VFS Integration**: Tests use the actual VFS service with IndexedDB storage
- **Sync Middleware**: Tests real-world sync scenarios using Zustand middleware
- **Bundle Operations**: Tests upload/download workflows and post-upload sync issues

## Next Steps

1. Install dependencies: `pnpm install`
2. Create actual test specifications in `tests/` directories
3. Run tests to verify setup

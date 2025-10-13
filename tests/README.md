# Tonk Test Suite

Comprehensive Playwright test suite for the Tonk relay server, testing WebSocket sync, bundle
operations, IndexedDB storage, and long-running stability.

## Setup

```bash
cd /Users/jackdouglas/tonk/tonk/tests
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
pnpm test:uptime    # Uptime and stability tests (long-running)

# Quick uptime test (shorter duration)
pnpm test:uptime:quick

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
- `tests/uptime/` - **Long-running uptime and stability tests**

## Uptime Tests

Uptime tests validate server stability, performance degradation, and memory leaks over extended
periods (15-30+ minutes).

### Test Files

#### `extended-stability.spec.ts`

- **Duration**: 15-30 minutes
- **Purpose**: Validate stable server operation under steady-state load
- **Tests**:
  - 10 clients running for 30 minutes with continuous operations
  - 20 clients running for 15 minutes
- **Metrics**: Memory growth, latency stability, error rates, health score

#### `gradual-load-increase.spec.ts`

- **Duration**: 20-30 minutes
- **Purpose**: Test server behavior under gradually increasing load
- **Tests**:
  - Phased load increase: 1 → 5 → 15 → 30 → 50 clients
  - Latency tracking across load phases
- **Metrics**: Connection establishment time, per-client memory, throughput

#### `throttling-stress.spec.ts`

- **Duration**: 15-20 minutes
- **Purpose**: Test resilience under poor network conditions
- **Tests**:
  - Network latency throttling (50ms, 100ms, 250ms)
  - Intermittent disconnection/reconnection
- **Metrics**: Reconnection success rate, state consistency

#### `memory-leak-detection.spec.ts`

- **Duration**: 15-20 minutes
- **Purpose**: Detect memory leaks through cyclic operations
- **Tests**:
  - Cyclic connect/disconnect (5 clients/minute for 20 minutes)
  - Sustained load memory monitoring (10 clients for 15 minutes)
- **Metrics**: Memory growth rate, baseline memory recovery

### Uptime Test Output

After running uptime tests, detailed reports are saved to:

```
test-results/uptime-metrics/
  extended-stability-2025-10-13-14-30.csv
  extended-stability-2025-10-13-14-30.json
  extended-stability-2025-10-13-14-30.txt
  gradual-load-increase-2025-10-13-15-00.csv
  ...
```

**CSV Format** (for trend analysis in Excel/Sheets):

```csv
Timestamp,Elapsed (s),Connections,Total Operations,Ops/sec,Latency P95 (ms),Error Rate (%),Memory Used (MB)
1697205000000,30.0,10,300,10.0,45.2,0.0,125.5
1697205030000,60.0,10,600,10.0,46.1,0.0,126.2
...
```

**Text Report** (console output):

```
═══════════════════════════════════════════════════════════
  UPTIME TEST REPORT: extended-stability-30min
═══════════════════════════════════════════════════════════

DURATION: 1800.0s (30.0 minutes)
SNAPSHOTS: 60

OPERATIONS:
  Total Operations:     1800
  Total Errors:         0
  Error Rate:           0.00%

LATENCY:
  Average:              45.32ms
  P95:                  52.10ms
  P99:                  58.40ms

CONNECTIONS:
  Max Connections:      10
  Avg Connections:      10.0

MEMORY:
  Memory Growth:        15.23MB
  ✓ Memory growth acceptable

PERFORMANCE:
  Degradation Detected: ✓ NO
  Health Score:         95/100 🟢
```

### Running Uptime Tests

**Note**: Uptime tests take 15-30+ minutes to complete. Run them when you have time!

```bash
# Run all uptime tests (can take 1-2 hours total)
pnpm test:uptime

# Run a single uptime test
pnpm test:uptime:quick  # ~12-15 minutes

# Run specific test file
npx playwright test tests/uptime/extended-stability.spec.ts
```

## Key Features

- **Isolated Test Servers**: Each test spins up its own relay server on a random port
- **VFS Integration**: Tests use the actual VFS service with IndexedDB storage
- **Sync Middleware**: Tests real-world sync scenarios using Zustand middleware
- **Bundle Operations**: Tests upload/download workflows and post-upload sync issues
- **Uptime Monitoring**: Continuous metrics collection with CSV/JSON export
- **Memory Leak Detection**: Automated detection of memory growth patterns
- **Network Throttling**: Simulated poor network conditions
- **Connection Management**: Manages dozens of concurrent browser contexts

## Utilities

### `MetricsCollector`

- Collects performance metrics (latency, throughput, memory, errors)
- Calculates percentiles (P50, P95, P99)
- Exports CSV/JSON reports

### `ConnectionManager`

- Manages multiple browser contexts and pages
- Executes operations across connections
- Tracks connection health and statistics

### `ServerProfiler`

- Monitors server-side memory usage
- Tracks memory per connection and per operation
- Detects memory leaks on the server

### `UptimeLogger`

- Orchestrates continuous metrics logging
- Generates time-series data for trend analysis
- Calculates health scores and detects degradation
- Exports comprehensive reports (CSV, JSON, TXT)

## Next Steps

1. Install dependencies: `pnpm install`
2. Run quick tests: `pnpm test:stress`
3. Run uptime tests when ready: `pnpm test:uptime:quick`

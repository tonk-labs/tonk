# Uptime Tests - Quick Reference Guide

## Overview

The uptime test suite validates Tonk server stability, performance, and memory management over
extended periods. Tests run from 15-30+ minutes and generate detailed metrics for trend analysis.

## Quick Start

```bash
# Run all uptime tests (~1-2 hours total)
pnpm test:uptime

# Run quick version (~12-15 minutes)
pnpm test:uptime:quick

# Run specific test
npx playwright test tests/uptime/extended-stability.spec.ts
```

## Test Suite

### 1. Extended Stability Test (`extended-stability.spec.ts`)

**Duration**: 15-30 minutes  
**Purpose**: Validate server stability under steady-state load

**Tests**:

- **30-minute stability** - 10 clients, continuous operations
- **20-client test** - 20 clients for 15 minutes

**Success Criteria**:

- ✓ Error rate < 1%
- ✓ No performance degradation
- ✓ Memory growth < 100 MB
- ✓ Health score > 80/100
- ✓ All connections remain healthy

---

### 2. Gradual Load Increase (`gradual-load-increase.spec.ts`)

**Duration**: 20-30 minutes  
**Purpose**: Test server behavior under increasing load

**Load Phases**:

1. **Baseline** (5 min): 1 client
2. **Light** (5 min): 5 clients
3. **Medium** (5 min): 15 clients
4. **Heavy** (5 min): 30 clients
5. **Stress** (5 min): 50 clients
6. **Cool Down** (5 min): 50 clients, no operations

**Success Criteria**:

- ✓ Error rate < 5%
- ✓ P95 latency < 500ms at all phases
- ✓ Connection time < 5s even at 50 clients
- ✓ Memory per connection variance < 30%
- ✓ Health score > 60/100

---

### 3. Throttling Stress Test (`throttling-stress.spec.ts`)

**Duration**: 15-20 minutes  
**Purpose**: Test resilience under poor network conditions

**Scenarios**:

1. **Baseline** - No throttling
2. **Light Latency** - 50ms network delay
3. **Medium Latency** - 100ms network delay
4. **High Latency** - 250ms network delay
5. **Intermittent Disconnections** - Random disconnect/reconnect

**Success Criteria**:

- ✓ All operations eventually succeed
- ✓ State remains consistent
- ✓ Automatic reconnection works
- ✓ Error rate < 10%
- ✓ Health score > 50/100

---

### 4. Memory Leak Detection (`memory-leak-detection.spec.ts`)

**Duration**: 15-20 minutes  
**Purpose**: Detect memory leaks through cyclic operations

**Tests**:

- **Cyclic connections** - Connect/disconnect 5 clients every minute for 20 minutes
- **Sustained load** - 10 clients with continuous operations for 15 minutes

**Success Criteria**:

- ✓ Memory increase < 100 MB
- ✓ No sustained memory leak detected
- ✓ Memory returns to baseline after disconnects
- ✓ Error rate < 5%
- ✓ Heap growth rate < 1 MB/min

---

## Metrics & Reports

### Output Location

```
test-results/uptime-metrics/
  extended-stability-2025-10-13-14-30.csv
  extended-stability-2025-10-13-14-30.json
  extended-stability-2025-10-13-14-30.txt
  gradual-load-increase-2025-10-13-15-00.csv
  ...
```

### CSV Format

Perfect for importing into Excel/Google Sheets for trend analysis:

```csv
Timestamp,Elapsed (s),Connections,Total Operations,Ops/sec,Latency P95 (ms),Error Rate (%),Memory Used (MB)
1697205000000,30.0,10,300,10.0,45.2,0.0,125.5
1697205030000,60.0,10,600,10.0,46.1,0.0,126.2
```

### JSON Format

Structured data for programmatic analysis:

```json
{
  "testName": "extended-stability-30min",
  "startTime": 1697205000000,
  "endTime": 1697206800000,
  "durationSeconds": 1800,
  "snapshots": [...],
  "summary": {
    "totalOperations": 1800,
    "totalErrors": 0,
    "errorRate": 0,
    "avgLatency": 45.32,
    "p95Latency": 52.10,
    "p99Latency": 58.40,
    "maxConnections": 10,
    "memoryGrowthMB": 15.23,
    "degradationDetected": false,
    "healthScore": 95
  }
}
```

### Text Report

Human-readable console output:

```
═══════════════════════════════════════════════════════════
  UPTIME TEST REPORT: extended-stability-30min
═══════════════════════════════════════════════════════════

DURATION: 1800.0s (30.0 minutes)

OPERATIONS:
  Total Operations:     1800
  Total Errors:         0
  Error Rate:           0.00%

LATENCY:
  Average:              45.32ms
  P95:                  52.10ms
  P99:                  58.40ms

MEMORY:
  Memory Growth:        15.23MB
  ✓ Memory growth acceptable

PERFORMANCE:
  Degradation Detected: ✓ NO
  Health Score:         95/100 🟢
```

---

## Understanding Health Score

Health score is calculated from:

- **Error rate** - Lower is better (0% = best)
- **Performance degradation** - P95 latency increase < 20%
- **Memory growth** - < 50 MB normal, < 100 MB acceptable

**Score Interpretation**:

- 🟢 **90-100**: Excellent - Production ready
- 🟡 **70-89**: Good - Minor issues detected
- 🔴 **< 70**: Poor - Investigate issues

---

## Key Metrics Explained

### Latency Percentiles

- **P50 (Median)**: 50% of operations complete within this time
- **P95**: 95% of operations complete within this time (key SLA metric)
- **P99**: 99% of operations complete within this time (worst-case tracking)

### Memory Tracking

- **Client Memory**: Browser heap usage (JavaScript + WASM)
- **Server Memory**: Node.js RSS (Resident Set Size)
- **Memory Growth**: Difference between first and last quarter of snapshots

### Performance Degradation

Detected when P95 latency increases by > 20% from baseline to final phase.

### Error Rate

```
Error Rate = (Total Errors / Total Operations) × 100%
```

- < 1%: Excellent
- 1-5%: Acceptable
- > 5%: Needs investigation

---

## Troubleshooting

### Test Timeouts

If tests timeout, the server may not have started properly. Check:

```bash
# Verify relay server can start
cd packages/relay
pnpm run dev
```

### High Memory Growth

If memory growth > 100 MB:

1. Check for event listener leaks
2. Verify WebSocket cleanup
3. Review IndexedDB cleanup
4. Run GC explicitly in tests

### High Error Rates

If error rate > 5%:

1. Check server logs in test output
2. Verify network stability
3. Increase operation timeouts
4. Review server error handling

### Performance Degradation

If degradation detected:

1. Check for connection pool exhaustion
2. Verify database cleanup between operations
3. Monitor server CPU usage
4. Review operation batching logic

---

## Advanced Usage

### Custom Test Duration

Edit test files to adjust duration:

```typescript
const durationMinutes = 30; // Change to desired duration
```

### Custom Metrics Intervals

Adjust snapshot frequency:

```typescript
uptimeLogger.startLogging(30000); // 30 seconds (default)
uptimeLogger.startLogging(60000); // 1 minute (less frequent)
```

### Running Single Test

```bash
npx playwright test tests/uptime/extended-stability.spec.ts --timeout=3600000
```

### Debugging Tests

```bash
npx playwright test tests/uptime/extended-stability.spec.ts --headed --debug
```

---

## Best Practices

1. **Run uptime tests overnight or during off-hours** - They take 1-2 hours
2. **Monitor system resources** - Ensure adequate RAM/CPU available
3. **Close other browser tabs** - Reduce resource contention
4. **Review reports after each run** - Track trends over time
5. **Baseline first** - Run tests before making changes to establish baseline
6. **Compare reports** - Use CSV exports to compare runs in spreadsheets

---

## Architecture

### UptimeLogger

Orchestrates all uptime testing:

- Continuous metrics collection
- Time-series data export
- Health score calculation
- Performance degradation detection

### MetricsCollector

Tracks client-side metrics:

- Operation latency (P50, P95, P99)
- Throughput (ops/sec, bytes/sec)
- Browser memory usage
- Error tracking

### ServerProfiler

Tracks server-side metrics:

- Node.js memory (RSS, heap)
- Memory per connection
- Memory per operation
- Memory leak detection

### ConnectionManager

Manages test clients:

- Creates/destroys browser contexts
- Executes operations across clients
- Tracks connection health
- Provides connection statistics

---

## Next Steps

1. Run quick uptime test: `pnpm test:uptime:quick`
2. Review generated reports in `test-results/uptime-metrics/`
3. Import CSV into Excel/Sheets for trend analysis
4. Run full uptime suite before production deployment
5. Establish baseline metrics for your environment
6. Set up regular uptime test runs (weekly/monthly)

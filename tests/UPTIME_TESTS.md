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

- **Server Memory**: Node.js RSS (Resident Set Size) - fetched from `/metrics` endpoint
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

### ConnectionManager

Manages test clients:

- Creates/destroys browser contexts
- Executes operations across clients
- Tracks connection health
- Provides connection statistics

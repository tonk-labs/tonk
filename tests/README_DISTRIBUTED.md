# Distributed Load Testing for Tonk Relay

This directory contains a complete distributed load testing system for profiling the Tonk relay
server under high loads (100-2000+ concurrent connections).

## What This Is

A production-ready system that:

- Automatically provisions EC2 worker instances
- Deploys headless Playwright browsers to each worker
- Generates realistic load with thousands of concurrent WebSocket connections
- Collects comprehensive metrics from relay + all workers
- Detects performance anomalies (latency spikes, memory leaks, connection drops)
- Generates detailed reports (JSON, CSV, human-readable logs)
- Automatically cleans up all AWS resources

## Why You Need This

Running 1000+ headless browser instances locally requires ~150GB of RAM. This system distributes the
load across multiple EC2 instances, making it possible to stress-test your relay server at scale
without crashing your local machine.

## Quick Start

### 1. Prerequisites

```bash
# Configure AWS credentials
aws configure

# Install dependencies
cd tests
pnpm install
```

### 2. Run Your First Test

```bash
# Baseline test: 100 connections, 20 minutes, ~$0.10
npm run test:distributed:baseline
```

### 3. View Results

```bash
# Results saved to:
ls test-results/distributed/baseline-*/

# Files generated:
# - summary.json    (complete test results)
# - metrics.csv     (time-series data for Excel)
# - metrics.json    (raw metrics)
# - test.log        (human-readable summary)
```

## Available Tests

| Command                             | Connections | Workers | Duration | Cost   |
| ----------------------------------- | ----------- | ------- | -------- | ------ |
| `npm run test:distributed:baseline` | 100         | 2       | ~20 min  | ~$0.10 |
| `npm run test:distributed:moderate` | 500         | 3       | ~45 min  | ~$0.50 |
| `npm run test:distributed:heavy`    | 1,000       | 5       | ~90 min  | ~$1.70 |
| `npm run test:distributed:stress`   | 2,000       | 10      | ~90 min  | ~$2.70 |

## How It Works

```
Your Machine (Orchestrator)
    ↓
    ├─ Provisions EC2 Workers (AWS SDK)
    ├─ Deploys Test Scripts (SSH)
    ├─ Coordinates Test Phases (HTTP API)
    ├─ Collects Metrics (Real-time)
    └─ Generates Reports

EC2 Workers (5 instances)
    ↓
    ├─ Worker 1: 200 headless browsers → Relay Server
    ├─ Worker 2: 200 headless browsers → Relay Server
    ├─ Worker 3: 200 headless browsers → Relay Server
    ├─ Worker 4: 200 headless browsers → Relay Server
    └─ Worker 5: 200 headless browsers → Relay Server
         = 1000 total connections

Relay Server (big-relay on EC2)
    ↓
    Handles 1000+ concurrent WebSocket connections
    Reports metrics via /metrics endpoint
```

## Documentation

**Start here:**

- 📋 [PRE_FLIGHT_CHECKLIST.md](./PRE_FLIGHT_CHECKLIST.md) - Complete before first test
- 🚀 [QUICKSTART_DISTRIBUTED.md](./QUICKSTART_DISTRIBUTED.md) - Quick start guide

**Deep dive:**

- 🏗️ [DISTRIBUTED_LOAD_TESTING.md](./DISTRIBUTED_LOAD_TESTING.md) - Complete architecture
- 📊 [DISTRIBUTED_IMPLEMENTATION_SUMMARY.md](./DISTRIBUTED_IMPLEMENTATION_SUMMARY.md) -
  Implementation details

## Architecture

### Components

1. **Orchestrator** (Your Local Machine)
   - Provisions EC2 infrastructure via AWS SDK
   - Runs worker coordinator HTTP server
   - Aggregates metrics from all sources
   - Generates comprehensive reports
   - Cleans up all resources

2. **Worker Instances** (EC2)
   - Run headless Playwright browsers
   - Connect to relay server via WebSocket
   - Execute operations (button clicks, updates)
   - Report metrics to orchestrator
   - Auto-configured via SSH

3. **Relay Server** (big-relay EC2)
   - Your relay under test
   - Handles WebSocket connections
   - Exposes /metrics endpoint
   - Memory: 16GB (upgradable if needed)

### Test Phases

1. **Provision** (~5 min) - Create EC2 instances
2. **Deploy** (~3 min) - Install dependencies, start workers
3. **Warmup** (2-5 min) - Verify infrastructure health
4. **Ramp-Up** (5-15 min) - Gradually increase connections
5. **Sustained Load** (10-60 min) - Maintain target load
6. **Stress** (optional) - Push until relay breaks
7. **Cooldown** (2-5 min) - Stop operations gracefully
8. **Report** (~1 min) - Generate comprehensive reports
9. **Teardown** (~1 min) - Clean up all AWS resources

## Metrics Collected

### From Relay Server

- Active WebSocket connections
- Memory usage (RSS, heap)
- Process uptime
- CPU usage

### From Each Worker

- Connection count (total, healthy, failed)
- Operation latency (min, max, mean, p50, p95, p99)
- Operations completed/failed
- Memory usage per worker
- Error counts and types

### Aggregate Analysis

- Total connections across all workers
- Global latency percentiles
- Operations per second
- Error rate percentage
- Connection health score
- Memory growth detection
- Anomaly detection (spikes, leaks, drops)

## Cost Optimization

- **Spot Instances**: Default (70% cheaper than on-demand)
- **Auto-Cleanup**: Prevents orphaned resources
- **Cost Estimation**: Shown before test starts
- **Configurable Limits**: Set max cost in scenario config

## Safety Features

✅ **Automatic cleanup** - Runs even if test fails ✅ **Health monitoring** - Detects worker
failures ✅ **Graceful shutdown** - Closes connections properly ✅ **Manual cleanup** -
`npm run distributed:cleanup` ✅ **Cost limits** - Configurable per scenario ✅ **Timeout
protection** - Tests won't run forever

## Troubleshooting

### Test fails during provision

```bash
aws sts get-caller-identity  # Check credentials
```

### Workers don't register

```bash
ssh -i ~/.ssh/tonk-load-test.pem ec2-user@<worker-ip>
tail -f ~/worker/worker.log
```

### Manual cleanup needed

```bash
npm run distributed:cleanup
```

### Want to customize?

Edit `tests/distributed/test-scenarios.config.ts`

## Example Output

```
================================================================================
DISTRIBUTED LOAD TEST: HEAVY
================================================================================
Description: Heavy load test with 1000 connections
Workers: 5x c5.2xlarge
Total Connections: 1000
Relay: ec2-16-16-146-55.eu-north-1.compute.amazonaws.com:8080
================================================================================

💰 Estimated cost: $1.70 ($1.70/hour)

📦 PHASE: PROVISION
Provisioning 5 worker instances...
✓ All 5 instances are running

🚀 PHASE: DEPLOY
Deploying workers to 5 instances...
✓ All workers deployed

📈 Starting metrics collection...

🔥 PHASE: WARMUP
Duration: 5.0 minutes
Target Connections: 100
Operations/min: 30
Running phase...
  [45%] 2.8min remaining | Connections: 100/100 | Ops: 1234 | P95: 45ms | Errors: 0.0%

⚡ PHASE: SUSTAINED
Duration: 60.0 minutes
Target Connections: 1000
Operations/min: 60
Running phase...
  [75%] 15.0min remaining | Connections: 1000/1000 | Ops: 45678 | P95: 123ms | Errors: 0.1%

================================================================================
TEST SUMMARY
================================================================================
Peak Connections:    1000
Total Operations:    456789
Total Errors:        23
Error Rate:          0.05%
Avg Latency:         45.23ms
P95 Latency:         123.45ms
Memory Growth:       12.34MB
Duration:            89.23 minutes
================================================================================

Reports saved to: tests/test-results/distributed/heavy-1234567890/

✅ Test PASSED

🧹 PHASE: TEARDOWN
Cleaning up AWS resources...
✓ Teardown complete (90.12 minutes total)
```

## Next Steps

1. ✅ Complete [PRE_FLIGHT_CHECKLIST.md](./PRE_FLIGHT_CHECKLIST.md)
2. 🚀 Run baseline test to verify setup
3. 📊 Analyze results (import CSV to Excel)
4. ⚡ Run heavy test for 1000 connections
5. 💥 Run stress test to find relay limits
6. 🔧 Optimize relay based on findings

## Support

- GitHub Issues: https://github.com/tonk-labs/tonk/issues
- Full Docs: [DISTRIBUTED_LOAD_TESTING.md](./DISTRIBUTED_LOAD_TESTING.md)

---

**Status**: ✨ Ready for Testing

The distributed load testing system is complete and ready to profile your relay server under high
loads.

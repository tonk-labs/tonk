# Distributed Load Testing Architecture

This document describes the distributed load testing system for profiling the Tonk relay server
under high loads (up to 1000+ concurrent connections).

## Overview

The distributed testing system uses EC2 instances to distribute load generation across multiple
machines, avoiding the memory constraints of running hundreds of headless Playwright instances on a
single machine.

### Architecture Components

```
┌─────────────────────────────────────────────────────────────────┐
│                    ORCHESTRATOR (Local Machine)                  │
│  - Provisions worker EC2 instances via AWS SDK                   │
│  - Coordinates test phases                                       │
│  - Aggregates metrics from all sources                           │
│  - Generates final report                                        │
│  - Tears down infrastructure                                     │
└──────────┬──────────────────────────────────────────────────────┘
           │
           │ WebSocket Connections to Relay
           ↓
┌──────────────────────────────────────────────────────────────────┐
│   RELAY SERVER (big-relay - 16GB memory)                         │
│   ec2-16-16-146-55.eu-north-1.compute.amazonaws.com              │
│   - Rust relay binary                                            │
│   - /metrics endpoint for monitoring                             │
│   - WebSocket sync server                                        │
│   - Target: 1000+ concurrent connections                         │
└───────────────────────────────────────────────────────────────────┘
           ↑
           │ WebSocket Connections from Workers
           │
    ┌──────┴──────┬──────────┬──────────┬──────────┐
    │             │          │          │          │
┌───▼────┐  ┌────▼───┐  ┌───▼────┐  ┌──▼─────┐  ┌──▼─────┐
│Worker 1│  │Worker 2│  │Worker 3│  │Worker 4│  │Worker 5│
│200 conn│  │200 conn│  │200 conn│  │200 conn│  │200 conn│
│c5.2xl  │  │c5.2xl  │  │c5.2xl  │  │c5.2xl  │  │c5.2xl  │
│        │  │        │  │        │  │        │  │        │
│Headless│  │Headless│  │Headless│  │Headless│  │Headless│
│Playwrht│  │Playwrht│  │Playwrht│  │Playwrht│  │Playwrht│
└────┬───┘  └────┬───┘  └────┬───┘  └────┬───┘  └────┬───┘
     │           │           │           │           │
     └───────────┴───────────┴───────────┴───────────┘
                 │
                 ↓ HTTP Metrics API
        Orchestrator Coordinator Server
```

## File Structure

```
tests/
├── src/
│   ├── distributed/
│   │   ├── types.ts                   # Type definitions
│   │   ├── aws-infrastructure.ts      # EC2 provisioning/teardown
│   │   ├── worker-coordinator.ts      # HTTP server for worker comms
│   │   ├── metrics-aggregator.ts      # Collects metrics from all sources
│   │   └── load-generator-worker.ts   # Worker script (runs on EC2)
│   └── utils/
│       ├── connection-manager.ts      # Existing - manages connections
│       └── metrics-collector.ts       # Existing - collects metrics
├── tests/
│   └── distributed/
│       ├── distributed-stress.spec.ts # Main orchestrator test
│       └── test-scenarios.config.ts   # Test scenario configurations
└── scripts/
    └── distributed/
        ├── setup-worker.sh           # Worker EC2 setup script
        └── worker.service            # Systemd service for worker
```

## Component Descriptions

### 1. Orchestrator (Local Machine)

**File**: `tests/distributed/distributed-stress.spec.ts`

The orchestrator runs as a Playwright test on your local machine and:

- Provisions worker EC2 instances using AWS SDK
- Starts worker coordinator HTTP server locally
- Deploys worker scripts to EC2 instances via SSH
- Coordinates test phases (warmup, ramp-up, sustained load, stress test)
- Aggregates metrics from relay + all workers
- Generates comprehensive performance reports
- Tears down all infrastructure on completion/failure

### 2. Worker Coordinator Server

**File**: `src/distributed/worker-coordinator.ts`

HTTP server that runs on the orchestrator (local machine):

- `POST /worker/register` - Workers register on startup
- `POST /worker/metrics` - Workers push metrics
- `POST /worker/health` - Worker health check
- `GET /phase` - Workers poll for current test phase
- Broadcasts phase transitions to all workers
- Aggregates real-time metrics from all workers

### 3. Load Generator Worker

**File**: `src/distributed/load-generator-worker.ts`

Standalone Node script deployed to each EC2 worker instance:

- Accepts CLI args: coordinator URL, relay URL, connection count, worker ID
- Uses existing `ConnectionManager` to create headless Playwright connections
- Connects N clients to the relay server
- Executes operations (button clicks, updates) at specified rate
- Pushes metrics to coordinator via HTTP
- Responds to phase transitions from coordinator
- Runs in headless mode (no X server required)

### 4. Metrics Aggregator

**File**: `src/distributed/metrics-aggregator.ts`

Collects and analyzes metrics from multiple sources:

- Polls relay's `/metrics` endpoint (connections, memory, uptime)
- Receives worker metrics via coordinator
- Calculates aggregate statistics:
  - Total connections across all workers
  - P50/P95/P99 latency across all workers
  - Error rates
  - Memory usage trends
  - Connection health
- Detects anomalies (latency spikes, memory leaks, connection drops)
- Exports time-series data for visualization
- Generates final report with graphs and statistics

### 5. AWS Infrastructure Manager

**File**: `src/distributed/aws-infrastructure.ts`

Manages EC2 infrastructure lifecycle:

- Creates security groups (allows SSH, HTTP, outbound)
- Provisions worker EC2 instances (c5.2xlarge spot instances)
- Manages SSH key pairs
- Waits for instances to be ready
- Provides SSH connection helpers
- Graceful teardown with cleanup
- Cost estimation

### 6. Worker Setup Script

**File**: `scripts/distributed/setup-worker.sh`

Bash script that configures worker EC2 instances:

- Installs Node.js, pnpm
- Installs Playwright with system dependencies
- Installs Chromium for headless mode
- Copies worker script from orchestrator
- Installs npm dependencies
- Starts worker service

## Test Scenarios

**File**: `tests/distributed/test-scenarios.config.ts`

Pre-configured test scenarios:

### Baseline (100 connections)

- Workers: 2x c5.xlarge
- Connections: 50 per worker
- Duration: 10 minutes
- Operations: 30/min per client
- **Cost**: ~$0.40/hour (~$0.07 for 10min)

### Moderate (500 connections)

- Workers: 3x c5.2xlarge
- Connections: 167 per worker
- Duration: 30 minutes
- Operations: 60/min per client
- **Cost**: ~$1.00/hour (~$0.50 for 30min)

### Heavy (1000 connections)

- Workers: 5x c5.2xlarge
- Connections: 200 per worker
- Duration: 60 minutes
- Operations: 60/min per client
- **Cost**: ~$1.70/hour (~$1.70 for 60min)

### Extreme (1500 connections)

- Workers: 8x c5.2xlarge
- Connections: 188 per worker
- Duration: 60 minutes
- Operations: 120/min per client
- **Cost**: ~$2.70/hour (~$2.70 for 60min)

### Stress-to-Failure

- Workers: 10x c5.2xlarge
- Connections: Ramp from 100 to 2000
- Duration: Until relay breaks
- Operations: Increase until failure
- **Cost**: ~$3.40/hour (varies by failure time)

## Test Phases

Each test scenario executes these phases:

1. **Provision** (~5 min)
   - Create EC2 worker instances
   - Wait for instances to be running
   - Configure security groups

2. **Deploy** (~3 min)
   - SSH to each worker
   - Run setup script
   - Start worker processes
   - Wait for worker registration

3. **Warmup** (~2 min)
   - Connect 10% of target connections
   - Run light operations
   - Verify all workers healthy

4. **Ramp-Up** (~10 min)
   - Gradually increase to target connections
   - Monitor relay health
   - Detect early failures

5. **Sustained Load** (configurable: 10-60 min)
   - Maintain target connections
   - Execute operations at target rate
   - Continuous metric collection
   - Monitor for degradation

6. **Stress Test** (optional)
   - Increase operations/min gradually
   - Find breaking point
   - Measure maximum throughput

7. **Cool Down** (~2 min)
   - Stop operations
   - Maintain connections
   - Verify graceful handling

8. **Report Generation** (~1 min)
   - Aggregate all metrics
   - Generate charts and graphs
   - Export CSV/JSON data
   - Save to `test-results/distributed/`

9. **Teardown** (~1 min)
   - Disconnect all clients
   - Stop worker processes
   - Terminate EC2 instances
   - Clean up security groups

## Metrics Collected

### Relay Metrics (from `/metrics` endpoint)

- Active WebSocket connections
- Memory usage (RSS, heap)
- Uptime
- Process ID

### Worker Metrics (from each worker)

- Connections established
- Connections healthy/unhealthy
- Operation latencies (min/max/mean/p50/p95/p99)
- Operations completed
- Error count and types
- Memory usage per worker

### Aggregate Metrics (calculated)

- Total connections across all workers
- Global latency percentiles
- Total operations/second
- Error rate (%)
- Connection health score
- Memory growth rate
- Relay CPU usage (if available)

## Output Reports

Reports saved to `tests/test-results/distributed/<test-name>-<timestamp>/`:

- `summary.json` - High-level test summary
- `metrics.csv` - Time-series metrics data
- `relay-metrics.json` - Relay server metrics
- `worker-metrics.json` - Per-worker metrics
- `latency-distribution.json` - Latency histogram
- `report.html` - Visual report with charts
- `test.log` - Detailed execution log

## Usage

### Running a Test

```bash
# Run baseline test
npm run test:distributed:baseline

# Run moderate load test
npm run test:distributed:moderate

# Run heavy load test (1000 connections)
npm run test:distributed:heavy

# Run stress-to-failure test
npm run test:distributed:stress

# Run custom scenario
npm run test:distributed -- --scenario custom --workers 5 --connections 200
```

### Environment Variables

Required in `.env`:

```bash
# AWS Configuration
AWS_REGION=eu-north-1
AWS_PROFILE=default  # or your profile name

# Relay Configuration
RELAY_HOST=ec2-16-16-146-55.eu-north-1.compute.amazonaws.com
RELAY_PORT=8080

# Optional: SSH Key
AWS_KEY_NAME=tonk-load-test
AWS_KEY_PATH=~/.ssh/tonk-load-test.pem
```

### Prerequisites

1. AWS credentials configured (`aws configure`)
2. SSH key pair created in EC2 (or script will create one)
3. Relay server running on `big-relay` instance
4. Local machine with:
   - Node.js 18+
   - AWS CLI
   - pnpm
   - Sufficient bandwidth for metric collection

## Cost Estimation

### EC2 Instance Pricing (eu-north-1, on-demand)

- c5.xlarge: $0.192/hour
- c5.2xlarge: $0.384/hour
- c5.4xlarge: $0.768/hour

### Spot Instance Pricing (70% discount typical)

- c5.xlarge: ~$0.058/hour
- c5.2xlarge: ~$0.115/hour
- c5.4xlarge: ~$0.230/hour

### Example Test Costs (using spot instances)

- 10-min baseline test: ~$0.04
- 30-min moderate test: ~$0.17
- 60-min heavy test: ~$0.58
- Full day of testing: ~$5-10

**Note**: Network transfer costs are minimal (< $0.01 per test) since workers are in same region as
relay.

## Troubleshooting

### Workers fail to connect to relay

- Check security group allows outbound HTTPS/WSS
- Verify relay is running: `curl http://$RELAY_HOST:$RELAY_PORT`
- Check relay logs for connection errors

### Workers fail to provision

- Check AWS credentials: `aws sts get-caller-identity`
- Verify EC2 limits in your account
- Check spot instance availability in region

### High error rates during test

- Relay may be overloaded - reduce connection count
- Network issues - check inter-instance latency
- Worker out of memory - reduce connections per worker

### Tests don't clean up properly

- Manual cleanup: `npm run distributed:cleanup`
- Terminate instances: `aws ec2 terminate-instances --instance-ids <ids>`
- Delete security groups: `aws ec2 delete-security-group --group-id <id>`

## Future Enhancements

- [ ] Real-time web dashboard for monitoring
- [ ] Grafana integration for metrics visualization
- [ ] Automatic relay server provisioning
- [ ] Multi-region load testing
- [ ] Custom operation scenarios (not just button clicks)
- [ ] Video recording of test execution
- [ ] Slack/email notifications on test completion
- [ ] Historical test result comparison
- [ ] Auto-scaling workers based on target connections
- [ ] Kubernetes deployment option

## References

- Existing tests: `tests/uptime/gradual-load-increase.spec.ts`
- Connection Manager: `tests/src/utils/connection-manager.ts`
- Metrics Collector: `tests/src/utils/metrics-collector.ts`
- Relay Server: `packages/relay/`

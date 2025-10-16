# Quick Start: Distributed Load Testing

This guide will help you run your first distributed load test against the relay server.

## Prerequisites

1. **AWS Credentials**

   ```bash
   aws configure
   # Enter your AWS Access Key ID, Secret Access Key, and region (eu-north-1)
   ```

2. **SSH Key** (optional - will be auto-created if not exists)

   ```bash
   # If you want to use an existing key:
   export AWS_KEY_NAME=your-key-name
   export AWS_KEY_PATH=~/.ssh/your-key-name.pem
   ```

3. **Install Dependencies**
   ```bash
   cd tests
   pnpm install
   ```

## Running Your First Test

### Baseline Test (100 connections, ~20 min, ~$0.10)

```bash
cd tests
npm run test:distributed:baseline
```

This will:

- Provision 2 EC2 instances (c5.xlarge)
- Connect 100 total clients (50 per worker)
- Run for ~20 minutes
- Generate comprehensive reports
- Clean up all resources automatically

### What Happens

```
📦 PHASE: PROVISION (~5 min)
   ├─ Create SSH key pair
   ├─ Create security group
   ├─ Launch 2 EC2 instances
   └─ Wait for SSH ready

🚀 PHASE: DEPLOY (~3 min)
   ├─ Install Node.js and dependencies
   ├─ Copy worker scripts
   ├─ Start worker processes
   └─ Wait for worker registration

🔥 PHASE: WARMUP (2 min)
   └─ Connect 10 clients, verify health

📈 PHASE: RAMPUP (5 min)
   └─ Gradually increase to 100 connections

⚡ PHASE: SUSTAINED (10 min)
   └─ Maintain 100 connections with operations

❄️ PHASE: COOLDOWN (2 min)
   └─ Stop operations, keep connections alive

📊 PHASE: REPORT (~1 min)
   ├─ Aggregate metrics from all workers
   ├─ Detect anomalies
   ├─ Generate CSV/JSON reports
   └─ Save to test-results/distributed/

🧹 PHASE: TEARDOWN (~1 min)
   ├─ Stop all workers
   ├─ Terminate EC2 instances
   └─ Delete security groups
```

## Available Test Scenarios

### 1. Baseline (100 connections)

```bash
npm run test:distributed:baseline
```

- **Workers**: 2x c5.xlarge
- **Duration**: ~20 minutes
- **Cost**: ~$0.10

### 2. Moderate (500 connections)

```bash
npm run test:distributed:moderate
```

- **Workers**: 3x c5.2xlarge
- **Duration**: ~45 minutes
- **Cost**: ~$0.50

### 3. Heavy (1000 connections)

```bash
npm run test:distributed:heavy
```

- **Workers**: 5x c5.2xlarge
- **Duration**: ~90 minutes
- **Cost**: ~$1.70

### 4. Stress (up to 2000 connections)

```bash
npm run test:distributed:stress
```

- **Workers**: 10x c5.2xlarge
- **Duration**: ~90 minutes
- **Cost**: ~$2.70
- **Goal**: Find relay breaking point

## Understanding Results

After the test completes, you'll see a summary:

```
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
```

### Report Files

- **summary.json** - Complete test results
- **metrics.csv** - Time-series data for graphing
- **metrics.json** - Raw metrics from all workers
- **test.log** - Human-readable summary

## Monitoring During Test

While the test runs, you'll see real-time progress:

```
  [45%] 5.5min remaining | Connections: 1000/1000 | Ops: 45000 | P95: 123ms | Errors: 0.1%
```

## Troubleshooting

### Test fails during provision

```bash
# Check AWS credentials
aws sts get-caller-identity

# Check EC2 limits
aws service-quotas get-service-quota \
  --service-code ec2 \
  --quota-code L-1216C47A
```

### Workers don't register

```bash
# SSH into worker to check logs
ssh -i ~/.ssh/tonk-load-test.pem ec2-user@<worker-ip>
tail -f ~/worker/worker.log
```

### Test hangs or crashes

```bash
# Manual cleanup
npm run distributed:cleanup
```

### Want to customize?

Edit `tests/tests/distributed/test-scenarios.config.ts` to:

- Change connection counts
- Adjust phase durations
- Modify operations per minute
- Use different instance types

## Next Steps

1. **Analyze Results**
   - Import CSV into Excel/Google Sheets
   - Graph latency over time
   - Identify breaking points

2. **Optimize Relay**
   - Increase relay instance size if needed
   - Tune relay configuration
   - Add more relay servers

3. **Scale Up**
   - Run stress test to find limits
   - Test with different operation patterns
   - Simulate real-world usage

## Cost Management

- Tests use **spot instances** by default (70% cheaper)
- Automatic cleanup prevents orphaned resources
- Set `maxCostPerHour` in scenario config as safety limit

## Support

- Full docs: `tests/DISTRIBUTED_LOAD_TESTING.md`
- Report issues: https://github.com/tonk-labs/tonk/issues
- Manual cleanup: `npm run distributed:cleanup`

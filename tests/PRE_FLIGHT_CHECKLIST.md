# Pre-Flight Checklist

Before running your first distributed load test, complete these steps:

## ✅ 1. AWS Configuration

```bash
# Check AWS credentials are configured
aws sts get-caller-identity
```

**Expected output:**

```json
{
  "UserId": "...",
  "Account": "...",
  "Arn": "arn:aws:iam::..."
}
```

**If not configured:**

```bash
aws configure
# Enter your AWS Access Key ID
# Enter your AWS Secret Access Key
# Default region: eu-north-1
# Default output format: json
```

## ✅ 2. Verify Relay Server

```bash
# Test relay is accessible
curl http://ec2-16-16-146-55.eu-north-1.compute.amazonaws.com:8080

# Check relay metrics endpoint
curl http://ec2-16-16-146-55.eu-north-1.compute.amazonaws.com:8080/metrics
```

**Expected:** Both should return successful responses.

## ✅ 3. Install Dependencies

```bash
cd tests
pnpm install
```

**Expected:** All dependencies install without errors.

## ✅ 4. Set Environment Variables (Optional)

```bash
# Optional: Use custom relay
export RELAY_HOST=your-relay-host.com
export RELAY_PORT=8080

# Optional: Use existing SSH key
export AWS_KEY_NAME=your-existing-key
export AWS_KEY_PATH=~/.ssh/your-key.pem

# Optional: Use custom AWS region
export AWS_REGION=eu-north-1
```

## ✅ 5. Verify EC2 Service Limits

```bash
# Check your EC2 instance limits
aws service-quotas get-service-quota \
  --service-code ec2 \
  --quota-code L-1216C47A \
  --region eu-north-1
```

**Minimum required:**

- Baseline: 2 instances
- Moderate: 3 instances
- Heavy: 5 instances
- Stress: 10 instances

## ✅ 6. Cost Awareness

Review estimated costs before running:

| Test     | Workers | Duration | Spot Cost | On-Demand |
| -------- | ------- | -------- | --------- | --------- |
| Baseline | 2       | 20 min   | ~$0.10    | ~$0.30    |
| Moderate | 3       | 45 min   | ~$0.50    | ~$1.50    |
| Heavy    | 5       | 90 min   | ~$1.70    | ~$5.00    |
| Stress   | 10      | 90 min   | ~$2.70    | ~$9.00    |

## ✅ 7. Dry Run (Recommended)

Run the baseline test first to verify everything works:

```bash
npm run test:distributed:baseline
```

This will:

- Test your AWS credentials
- Verify SSH key creation
- Test EC2 provisioning
- Verify worker deployment
- Test relay connectivity
- Generate sample reports

**Duration:** ~20 minutes **Cost:** ~$0.10

## ✅ 8. Monitor During Test

Once started, you'll see real-time output:

```
📦 PHASE: PROVISION (~5 min)
🚀 PHASE: DEPLOY (~3 min)
🔥 PHASE: WARMUP (2 min)
📈 PHASE: RAMPUP (5 min)
  [45%] 2.8min remaining | Connections: 100/100 | Ops: 1234 | P95: 45ms | Errors: 0.0%
⚡ PHASE: SUSTAINED (10 min)
❄️ PHASE: COOLDOWN (2 min)
📊 PHASE: REPORT (~1 min)
🧹 PHASE: TEARDOWN (~1 min)
```

## ✅ 9. Emergency Cleanup

If test fails or hangs, run manual cleanup:

```bash
npm run distributed:cleanup
```

This will find and terminate all test EC2 instances.

## ✅ 10. Review Results

After test completes, check:

```bash
# View latest test results
ls -lt tests/test-results/distributed/

# View summary
cat tests/test-results/distributed/baseline-*/test.log

# Open CSV in Excel
open tests/test-results/distributed/baseline-*/metrics.csv
```

## Ready to Go! 🚀

If all checklist items pass, you're ready to run your first test:

```bash
cd tests
npm run test:distributed:baseline
```

## Common Issues

### "Access Denied" errors

- Check AWS credentials: `aws configure`
- Verify IAM permissions for EC2

### "Spot instance request failed"

- Try on-demand instances (edit scenario config)
- Check availability zone

### Workers don't register

- SSH into worker: `ssh -i ~/.ssh/tonk-load-test.pem ec2-user@<ip>`
- Check logs: `tail -f ~/worker/worker.log`

### Test hangs

- Press Ctrl+C (cleanup runs automatically)
- Or run: `npm run distributed:cleanup`

## Need Help?

- Full docs: `tests/DISTRIBUTED_LOAD_TESTING.md`
- Quick start: `tests/QUICKSTART_DISTRIBUTED.md`
- Implementation: `tests/DISTRIBUTED_IMPLEMENTATION_SUMMARY.md`

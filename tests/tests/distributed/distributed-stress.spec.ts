import { test, expect } from '@playwright/test';
import { AWSInfrastructure } from '../../src/distributed/aws-infrastructure';
import { MetricsAggregator } from '../../src/distributed/metrics-aggregator';
import { getScenario } from './test-scenarios.config';
import type {
  EC2Config,
  ProvisionedInstance,
  TestPhase,
} from '../../src/distributed/types';
import { execSync } from 'child_process';
import { join } from 'path';
import { promises as fs } from 'fs';

const AWS_REGION = process.env.AWS_REGION || 'eu-north-1';
const AWS_KEY_NAME = process.env.AWS_KEY_NAME || 'tonk-load-test';

test.describe('Distributed Load Testing', () => {
  test('baseline - 100 connections @baseline', async () => {
    await runDistributedTest('baseline');
  });

  test('moderate - 500 connections @moderate', async () => {
    await runDistributedTest('moderate');
  });

  test('heavy - 1000 connections @heavy', async () => {
    await runDistributedTest('heavy');
  });

  test('stress - push to failure @stress', async () => {
    await runDistributedTest('stress');
  });
});

async function runDistributedTest(scenarioName: string) {
  const scenario = getScenario(scenarioName);
  const testStartTime = Date.now();
  const testName = `${scenarioName}-${testStartTime}`;

  console.log('\n' + '='.repeat(80));
  console.log(`DISTRIBUTED LOAD TEST: ${scenario.name.toUpperCase()}`);
  console.log('='.repeat(80));
  console.log(`Description: ${scenario.description}`);
  console.log(
    `Workers: ${scenario.workerCount}x ${scenario.workerInstanceType}`
  );
  console.log(
    `Total Connections: ${scenario.workerCount * scenario.connectionsPerWorker}`
  );
  console.log(`Relay: ${scenario.relayHost}:${scenario.relayPort}`);
  console.log('='.repeat(80) + '\n');

  let infrastructure: AWSInfrastructure | undefined;
  let metricsAggregator: MetricsAggregator | undefined;
  let provisionedInstances: ProvisionedInstance[] = [];
  let coordinatorInstance: ProvisionedInstance | undefined;
  let coordinatorUrl: string | undefined;

  const phaseResults: {
    phase: TestPhase;
    startTime: number;
    endTime: number;
    duration: number;
    status: 'completed' | 'failed' | 'skipped';
  }[] = [];

  try {
    const relayUrl = `http://${scenario.relayHost}:${scenario.relayPort}`;

    console.log('📊 Initializing metrics aggregator...');
    metricsAggregator = new MetricsAggregator(relayUrl, testName);

    const estimatedDuration =
      scenario.phases.reduce((sum, p) => sum + p.durationMs, 0) / 1000 / 60;

    const ec2Config: EC2Config = {
      region: AWS_REGION,
      instanceType: scenario.workerInstanceType,
      keyName: AWS_KEY_NAME,
      securityGroupName: `tonk-load-test-${testName}`,
      useSpotInstances: scenario.useSpotInstances,
      maxSpotPrice: '0.50',
      instanceTags: {
        Name: `tonk-worker-${testName}`,
        Project: 'tonk-load-test',
        Scenario: scenarioName,
        ManagedBy: 'playwright-test',
      },
    };

    infrastructure = new AWSInfrastructure(ec2Config);

    const cost = await infrastructure.estimateCost(
      scenario.workerCount + 1,
      estimatedDuration / 60
    );
    console.log(
      `💰 Estimated cost: $${cost.total.toFixed(2)} ($${cost.perHour.toFixed(2)}/hour)`
    );

    console.log('\n📦 PHASE: PROVISION');
    const provisionStart = Date.now();

    await infrastructure.setupKeyPair();
    await infrastructure.setupSecurityGroup();

    console.log('\nProvisioning coordinator instance...');
    coordinatorInstance = await infrastructure.provisionCoordinator();

    console.log('Waiting for coordinator SSH to be ready...');
    await infrastructure.waitForSSHReady([coordinatorInstance]);

    console.log('\nDeploying coordinator...');
    await deployCoordinatorToInstance(
      coordinatorInstance,
      infrastructure.getKeyPath(),
      scenario
    );

    coordinatorUrl = `http://${coordinatorInstance.publicIp}:9000`;
    console.log(`Coordinator URL: ${coordinatorUrl}`);

    console.log('\nWaiting for coordinator to be ready...');
    await waitForCoordinatorReady(coordinatorUrl, 60000);

    console.log(`\nProvisioning ${scenario.workerCount} worker instances...`);
    provisionedInstances = await infrastructure.provisionWorkers(
      scenario.workerCount
    );

    console.log('Waiting for worker SSH to be ready...');
    await infrastructure.waitForSSHReady(provisionedInstances);

    phaseResults.push({
      phase: 'provision',
      startTime: provisionStart,
      endTime: Date.now(),
      duration: Date.now() - provisionStart,
      status: 'completed',
    });

    console.log('\n🚀 PHASE: DEPLOY');
    const deployStart = Date.now();

    await deployWorkersToInstances(
      provisionedInstances,
      infrastructure.getKeyPath(),
      coordinatorUrl,
      relayUrl,
      scenario.connectionsPerWorker
    );

    console.log('Waiting for workers to register...');
    await waitForWorkerRegistration(
      coordinatorUrl,
      scenario.workerCount,
      180000
    );

    phaseResults.push({
      phase: 'deploy',
      startTime: deployStart,
      endTime: Date.now(),
      duration: Date.now() - deployStart,
      status: 'completed',
    });

    console.log('\n📈 Starting metrics collection...');
    const workers = await getWorkersFromCoordinator(coordinatorUrl);
    metricsAggregator.startPolling(workers, 5000);

    for (const phaseConfig of scenario.phases) {
      console.log(
        `\n${getPhaseEmoji(phaseConfig.phase)} PHASE: ${phaseConfig.phase.toUpperCase()}`
      );
      console.log(
        `Duration: ${(phaseConfig.durationMs / 1000 / 60).toFixed(1)} minutes`
      );
      console.log(`Target Connections: ${phaseConfig.targetConnections}`);
      console.log(`Operations/min: ${phaseConfig.operationsPerMinute}`);

      const phaseStart = Date.now();

      const connectionsPerWorker = Math.ceil(
        phaseConfig.targetConnections / scenario.workerCount
      );

      console.log('Sending start command to workers...');
      const startCommands = await broadcastCommandToWorkers(coordinatorUrl, {
        type: 'start',
        payload: {
          targetConnections: connectionsPerWorker,
          operationsPerMinute: phaseConfig.operationsPerMinute,
        },
      });

      const successCount = startCommands.filter((r: any) => r.success).length;
      console.log(`✓ ${successCount}/${scenario.workerCount} workers started`);

      if (successCount < scenario.workerCount) {
        console.warn(`⚠️  Only ${successCount} workers started successfully`);
        startCommands.forEach((r: any) => {
          if (!r.success) {
            console.error(`  Worker ${r.workerId} failed: ${r.error}`);
          }
        });
      }

      console.log('Running phase...');
      const phaseEndTime = phaseStart + phaseConfig.durationMs;

      while (Date.now() < phaseEndTime) {
        await new Promise(resolve => setTimeout(resolve, 10000));

        const elapsed = Date.now() - phaseStart;
        const remaining = phaseEndTime - Date.now();
        const progress = (elapsed / phaseConfig.durationMs) * 100;

        const latestMetrics = metricsAggregator.getLatestMetrics();
        if (latestMetrics) {
          console.log(
            `  [${progress.toFixed(0)}%] ${(remaining / 1000 / 60).toFixed(1)}min remaining | ` +
              `Connections: ${latestMetrics.aggregate.totalConnections}/${phaseConfig.targetConnections} | ` +
              `Ops: ${latestMetrics.aggregate.totalOperations} | ` +
              `P95: ${latestMetrics.aggregate.latency.p95.toFixed(0)}ms | ` +
              `Errors: ${latestMetrics.aggregate.errorRate.toFixed(1)}%`
          );

          if (phaseConfig.phase === 'stress') {
            if (latestMetrics.aggregate.errorRate > 10) {
              console.log(
                '🔴 Error rate exceeded 10% - relay is breaking down'
              );
              break;
            }
            if (latestMetrics.aggregate.latency.p95 > 5000) {
              console.log('🔴 P95 latency exceeded 5s - relay is overloaded');
              break;
            }
          }
        }

        const health = await checkWorkerHealth(coordinatorUrl);
        if (health.stale.length > 0) {
          console.warn(`⚠️  Stale workers: ${health.stale.join(', ')}`);
        }
      }

      phaseResults.push({
        phase: phaseConfig.phase,
        startTime: phaseStart,
        endTime: Date.now(),
        duration: Date.now() - phaseStart,
        status: 'completed',
      });

      const phaseMetrics = metricsAggregator.getLatestMetrics();
      if (phaseMetrics) {
        console.log('Phase Summary:');
        console.log(
          `  Total Connections: ${phaseMetrics.aggregate.totalConnections}`
        );
        console.log(
          `  Total Operations: ${phaseMetrics.aggregate.totalOperations}`
        );
        console.log(
          `  Avg Latency: ${phaseMetrics.aggregate.latency.mean.toFixed(2)}ms`
        );
        console.log(
          `  P95 Latency: ${phaseMetrics.aggregate.latency.p95.toFixed(2)}ms`
        );
        console.log(
          `  Error Rate: ${phaseMetrics.aggregate.errorRate.toFixed(2)}%`
        );
      }
    }

    console.log('\n📊 PHASE: REPORT');
    const reportStart = Date.now();

    metricsAggregator.stopPolling();

    console.log('Generating report...');
    const report = await metricsAggregator.generateReport(
      scenarioName,
      phaseResults
    );

    console.log('Detecting anomalies...');
    const anomalies = await metricsAggregator.detectAnomalies();

    if (anomalies.latencySpikes.length > 0) {
      console.log(
        `⚠️  Detected ${anomalies.latencySpikes.length} latency spikes`
      );
    }

    if (anomalies.connectionDrops.length > 0) {
      console.log(
        `⚠️  Detected ${anomalies.connectionDrops.length} connection drops`
      );
    }

    if (anomalies.memoryLeaks.growthMB > 100) {
      console.log(
        `⚠️  Possible memory leak: +${anomalies.memoryLeaks.growthMB.toFixed(2)}MB`
      );
    }

    console.log('Saving report...');
    const savedFiles = await metricsAggregator.saveReport(report);

    phaseResults.push({
      phase: 'report',
      startTime: reportStart,
      endTime: Date.now(),
      duration: Date.now() - reportStart,
      status: 'completed',
    });

    console.log('\n' + '='.repeat(80));
    console.log('TEST SUMMARY');
    console.log('='.repeat(80));
    console.log(`Peak Connections:    ${report.summary.peakConnections}`);
    console.log(`Total Operations:    ${report.summary.totalOperations}`);
    console.log(`Total Errors:        ${report.summary.totalErrors}`);
    console.log(`Error Rate:          ${report.summary.errorRate.toFixed(2)}%`);
    console.log(
      `Avg Latency:         ${report.summary.avgLatency.toFixed(2)}ms`
    );
    console.log(
      `P95 Latency:         ${report.summary.p95Latency.toFixed(2)}ms`
    );
    console.log(
      `Memory Growth:       ${report.summary.relayMemoryGrowth.toFixed(2)}MB`
    );
    console.log(
      `Duration:            ${(report.duration / 1000 / 60).toFixed(2)} minutes`
    );
    console.log('='.repeat(80));
    console.log(
      `\nReports saved to: ${savedFiles[0].split('/').slice(0, -1).join('/')}`
    );

    expect(report.summary.errorRate).toBeLessThan(5);
    expect(report.summary.p95Latency).toBeLessThan(1000);

    console.log('\n✅ Test PASSED');
  } catch (error) {
    console.error('\n❌ Test FAILED:', error);

    if (infrastructure && provisionedInstances.length > 0) {
      try {
        await fetchWorkerLogs(
          provisionedInstances,
          infrastructure.getKeyPath()
        );
      } catch (logError) {
        console.error('Failed to fetch worker logs:', logError);
      }
    }

    if (infrastructure && coordinatorInstance) {
      try {
        await fetchCoordinatorLog(
          coordinatorInstance,
          infrastructure.getKeyPath()
        );
      } catch (logError) {
        console.error('Failed to fetch coordinator log:', logError);
      }
    }

    throw error;
  } finally {
    console.log('\n🧹 PHASE: TEARDOWN');
    const teardownStart = Date.now();

    if (coordinatorUrl && infrastructure && provisionedInstances.length > 0) {
      console.log('Stopping workers and Vite servers...');
      try {
        const workers = await getWorkersFromCoordinator(coordinatorUrl);
        const keyPath = infrastructure.getKeyPath();

        for (const worker of workers) {
          try {
            await fetch(`http://${worker.publicIp}:3000/command`, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ type: 'stop' }),
            });
          } catch (e) {
            console.warn(`Failed to stop worker ${worker.workerId}`);
          }
        }

        for (const instance of provisionedInstances) {
          try {
            const sshBase = `ssh -o StrictHostKeyChecking=no -i ${keyPath} ec2-user@${instance.publicIp}`;
            execSync(`${sshBase} "pkill -f 'vite|tsx' || true"`, {
              stdio: 'pipe',
            });
          } catch (e) {
            console.warn(`Failed to kill processes on ${instance.instanceId}`);
          }
        }

        await new Promise(resolve => setTimeout(resolve, 5000));
      } catch (error) {
        console.warn('Error stopping workers:', error);
      }
    }

    if (infrastructure && provisionedInstances.length > 0) {
      console.log('Cleaning up AWS resources...');
      const allInstanceIds = provisionedInstances.map(i => i.instanceId);
      if (coordinatorInstance) {
        allInstanceIds.push(coordinatorInstance.instanceId);
      }
      await infrastructure.cleanup(allInstanceIds);
    }

    phaseResults.push({
      phase: 'teardown',
      startTime: teardownStart,
      endTime: Date.now(),
      duration: Date.now() - teardownStart,
      status: 'completed',
    });

    const totalDuration = Date.now() - testStartTime;
    console.log(
      `\n✓ Teardown complete (${(totalDuration / 1000 / 60).toFixed(2)} minutes total)`
    );
  }
}

async function deployWorkersToInstances(
  instances: ProvisionedInstance[],
  keyPath: string,
  coordinatorUrl: string,
  relayUrl: string,
  connectionsPerWorker: number
): Promise<void> {
  console.log(`Deploying workers to ${instances.length} instances...`);

  const setupScriptPath = join(
    process.cwd(),
    'scripts/distributed/setup-worker.sh'
  );

  for (let i = 0; i < instances.length; i++) {
    const instance = instances[i];
    const workerId = `worker-${i}`;

    console.log(
      `\n[${i + 1}/${instances.length}] Deploying to ${instance.instanceId} (${instance.publicIp})...`
    );

    const sshBase = `ssh -o StrictHostKeyChecking=no -i ${keyPath} ec2-user@${instance.publicIp}`;

    try {
      console.log('  Creating worker directory structure...');
      execSync(`${sshBase} "mkdir -p ~/worker"`, { stdio: 'pipe' });

      console.log('  Copying setup script...');
      execSync(
        `scp -o StrictHostKeyChecking=no -i ${keyPath} ${setupScriptPath} ec2-user@${instance.publicIp}:~/`,
        { stdio: 'pipe' }
      );

      console.log('  Running setup script...');
      execSync(`${sshBase} "bash ~/setup-worker.sh"`, { stdio: 'inherit' });

      console.log('  Copying package.json...');
      const packageJsonPath = join(process.cwd(), 'package.json');
      execSync(
        `scp -o StrictHostKeyChecking=no -i ${keyPath} ${packageJsonPath} ec2-user@${instance.publicIp}:~/worker/package.json`,
        { stdio: 'pipe' }
      );

      console.log('  Copying source files...');
      const srcDir = join(process.cwd(), 'src');
      const testsDir = join(process.cwd(), 'tests');
      const rsyncIgnore = join(process.cwd(), '.rsyncignore');

      execSync(
        `rsync -av --exclude-from=${rsyncIgnore} -e "ssh -o StrictHostKeyChecking=no -i ${keyPath}" ${srcDir} ec2-user@${instance.publicIp}:~/worker/`,
        { stdio: 'inherit' }
      );
      execSync(
        `rsync -av --exclude-from=${rsyncIgnore} -e "ssh -o StrictHostKeyChecking=no -i ${keyPath}" ${testsDir} ec2-user@${instance.publicIp}:~/worker/`,
        { stdio: 'inherit' }
      );

      console.log('  Copying config files...');
      const configFiles = [
        'vite.config.ts',
        'vite.sw.config.ts',
        'tsconfig.json',
        'tsconfig.node.json',
      ];
      for (const configFile of configFiles) {
        const configPath = join(process.cwd(), configFile);
        execSync(
          `scp -o StrictHostKeyChecking=no -i ${keyPath} ${configPath} ec2-user@${instance.publicIp}:~/worker/${configFile}`,
          { stdio: 'pipe' }
        );
      }

      console.log('  Copying monorepo packages...');
      const packagesDir = join(process.cwd(), '../packages');
      execSync(`${sshBase} "mkdir -p ~/packages"`, { stdio: 'pipe' });

      const requiredPackages = ['host-web', 'core-js', 'core'];
      for (const pkg of requiredPackages) {
        const packagePath = join(packagesDir, pkg);
        execSync(
          `rsync -av --exclude-from=${rsyncIgnore} -e "ssh -o StrictHostKeyChecking=no -i ${keyPath}" ${packagePath} ec2-user@${instance.publicIp}:~/packages/`,
          { stdio: 'inherit' }
        );
      }

      console.log('  Installing dependencies...');
      execSync(`${sshBase} "cd ~/worker && pnpm install"`, {
        stdio: 'inherit',
      });

      console.log('  Installing Playwright browsers...');
      execSync(`${sshBase} "cd ~/worker && npx playwright install chromium"`, {
        stdio: 'inherit',
      });

      console.log('  Starting Vite dev server...');
      execSync(
        `${sshBase} -f "cd ~/worker && nohup pnpm dev --host 0.0.0.0 --port 5173 > vite.log 2>&1 &"`,
        { stdio: 'inherit' }
      );

      console.log('  Waiting for Vite server to be ready...');
      await waitForViteReady(instance.publicIp, 5173, 60000);

      console.log('  Starting worker...');
      const workerCmd = `cd ~/worker && nohup npx tsx src/distributed/load-generator-worker.ts ${workerId} ${coordinatorUrl} ${relayUrl} ${connectionsPerWorker} 60 ${instance.publicIp} > worker.log 2>&1 &`;
      execSync(`${sshBase} -f "${workerCmd}"`, { stdio: 'inherit' });

      console.log(`  ✓ Worker ${workerId} deployed and started`);
    } catch (error) {
      console.error(
        `  ✗ Failed to deploy worker to ${instance.instanceId}:`,
        error
      );
      throw error;
    }
  }

  console.log('\n✓ All workers deployed');
}

async function waitForWorkerRegistration(
  coordinatorUrl: string,
  expectedCount: number,
  timeoutMs: number
): Promise<void> {
  const startTime = Date.now();

  while (Date.now() - startTime < timeoutMs) {
    try {
      const response = await fetch(`${coordinatorUrl}/worker/status`);
      if (response.ok) {
        const data = await response.json();
        const activeWorkers = data.summary.totalWorkers;

        if (activeWorkers >= expectedCount) {
          console.log(`✓ All ${expectedCount} workers registered`);
          return;
        }

        console.log(
          `  Waiting for workers... (${activeWorkers}/${expectedCount})`
        );
      }
    } catch (error) {
      console.log(`  Waiting for coordinator response...`);
    }

    await new Promise(resolve => setTimeout(resolve, 5000));
  }

  try {
    const response = await fetch(`${coordinatorUrl}/worker/status`);
    const data = await response.json();
    throw new Error(
      `Timeout waiting for worker registration. Got ${data.summary.totalWorkers}/${expectedCount}`
    );
  } catch {
    throw new Error(
      `Timeout waiting for worker registration. Could not reach coordinator.`
    );
  }
}

async function deployCoordinatorToInstance(
  instance: ProvisionedInstance,
  keyPath: string,
  scenario: any
): Promise<void> {
  console.log(
    `Deploying coordinator to ${instance.instanceId} (${instance.publicIp})...`
  );

  const sshBase = `ssh -o StrictHostKeyChecking=no -i ${keyPath} ec2-user@${instance.publicIp}`;
  const setupScriptPath = join(
    process.cwd(),
    'scripts/distributed/setup-worker.sh'
  );

  try {
    console.log('  Creating coordinator directory structure...');
    execSync(`${sshBase} "mkdir -p ~/coordinator/src/distributed"`, {
      stdio: 'pipe',
    });

    console.log('  Copying setup script...');
    execSync(
      `scp -o StrictHostKeyChecking=no -i ${keyPath} ${setupScriptPath} ec2-user@${instance.publicIp}:~/`,
      { stdio: 'pipe' }
    );

    console.log('  Running setup script...');
    execSync(`${sshBase} "bash ~/setup-worker.sh"`, { stdio: 'inherit' });

    console.log('  Creating coordinator package.json...');
    const packageJson = {
      name: 'tonk-coordinator',
      type: 'module',
      dependencies: {
        express: '^4.21.2',
      },
    };
    const packageJsonContent = JSON.stringify(packageJson, null, 2);
    await fs.writeFile('/tmp/coordinator-package.json', packageJsonContent);
    execSync(
      `scp -o StrictHostKeyChecking=no -i ${keyPath} /tmp/coordinator-package.json ec2-user@${instance.publicIp}:~/coordinator/package.json`,
      { stdio: 'pipe' }
    );

    console.log('  Copying coordinator files...');
    const coordinatorScriptPath = join(
      process.cwd(),
      'src/distributed/coordinator-server.ts'
    );
    const workerCoordinatorPath = join(
      process.cwd(),
      'src/distributed/worker-coordinator.ts'
    );
    const typesPath = join(process.cwd(), 'src/distributed/types.ts');

    execSync(
      `scp -o StrictHostKeyChecking=no -i ${keyPath} ${coordinatorScriptPath} ec2-user@${instance.publicIp}:~/coordinator/src/distributed/coordinator-server.ts`,
      { stdio: 'pipe' }
    );
    execSync(
      `scp -o StrictHostKeyChecking=no -i ${keyPath} ${workerCoordinatorPath} ec2-user@${instance.publicIp}:~/coordinator/src/distributed/worker-coordinator.ts`,
      { stdio: 'pipe' }
    );
    execSync(
      `scp -o StrictHostKeyChecking=no -i ${keyPath} ${typesPath} ec2-user@${instance.publicIp}:~/coordinator/src/distributed/types.ts`,
      { stdio: 'pipe' }
    );

    console.log('  Installing dependencies...');
    execSync(`${sshBase} "cd ~/coordinator && pnpm install"`, {
      stdio: 'inherit',
    });

    console.log('  Starting coordinator...');
    const scenarioJson = Buffer.from(JSON.stringify(scenario)).toString(
      'base64'
    );
    const coordinatorCmd = `cd ~/coordinator && nohup npx tsx src/distributed/coordinator-server.ts 9000 ${scenarioJson} > coordinator.log 2>&1 &`;
    execSync(`${sshBase} -f "${coordinatorCmd}"`, { stdio: 'inherit' });

    console.log('  ✓ Coordinator deployed and started');
  } catch (error) {
    console.error(
      `  ✗ Failed to deploy coordinator to ${instance.instanceId}:`,
      error
    );
    throw error;
  }
}

async function waitForCoordinatorReady(
  coordinatorUrl: string,
  timeoutMs: number
): Promise<void> {
  const startTime = Date.now();

  while (Date.now() - startTime < timeoutMs) {
    try {
      const response = await fetch(`${coordinatorUrl}/health`);
      if (response.ok) {
        console.log('✓ Coordinator is ready');
        return;
      }
    } catch {
      // Continue waiting
    }
    await new Promise(resolve => setTimeout(resolve, 2000));
  }

  throw new Error('Timeout waiting for coordinator to become ready');
}

async function fetchWorkerLogs(
  instances: ProvisionedInstance[],
  keyPath: string
): Promise<void> {
  console.log('\n📋 Fetching worker logs for debugging...');

  for (const instance of instances) {
    try {
      const sshBase = `ssh -o StrictHostKeyChecking=no -i ${keyPath} ec2-user@${instance.publicIp}`;
      console.log(`\n${'='.repeat(80)}`);
      console.log(`Worker Log: ${instance.instanceId} (${instance.publicIp})`);
      console.log('='.repeat(80));

      try {
        const log = execSync(`${sshBase} "cat ~/worker/worker.log"`, {
          encoding: 'utf-8',
          maxBuffer: 10 * 1024 * 1024,
          stdio: ['pipe', 'pipe', 'pipe'],
        });
        console.log(log);
      } catch (error: any) {
        if (error.stdout) {
          console.log(error.stdout.toString());
        }
        console.error(`Error reading log: ${error.message}`);
      }
    } catch (error) {
      console.error(`Failed to fetch log from ${instance.instanceId}:`, error);
    }
  }
}

async function fetchCoordinatorLog(
  instance: ProvisionedInstance,
  keyPath: string
): Promise<void> {
  console.log('\n📋 Fetching coordinator log for debugging...');

  try {
    const sshBase = `ssh -o StrictHostKeyChecking=no -i ${keyPath} ec2-user@${instance.publicIp}`;
    console.log(`\n${'='.repeat(80)}`);
    console.log(
      `Coordinator Log: ${instance.instanceId} (${instance.publicIp})`
    );
    console.log('='.repeat(80));

    try {
      const log = execSync(`${sshBase} "cat ~/coordinator/coordinator.log"`, {
        encoding: 'utf-8',
        maxBuffer: 10 * 1024 * 1024,
        stdio: ['pipe', 'pipe', 'pipe'],
      });
      console.log(log);
    } catch (error: any) {
      if (error.stdout) {
        console.log(error.stdout.toString());
      }
      console.error(`Error reading log: ${error.message}`);
    }
  } catch (error) {
    console.error(
      `Failed to fetch coordinator log from ${instance.instanceId}:`,
      error
    );
  }
}

async function getWorkersFromCoordinator(
  coordinatorUrl: string
): Promise<any[]> {
  try {
    const response = await fetch(`${coordinatorUrl}/worker/status`);
    if (response.ok) {
      const data = await response.json();
      return data.workers || [];
    }
  } catch (error) {
    console.error('Failed to get workers from coordinator:', error);
  }
  return [];
}

async function broadcastCommandToWorkers(
  coordinatorUrl: string,
  command: any
): Promise<any[]> {
  try {
    const workers = await getWorkersFromCoordinator(coordinatorUrl);
    console.log(
      `Broadcasting ${command.type} command to ${workers.length} workers...`
    );

    const promises = workers.map(async (worker: any) => {
      const workerUrl = `http://${worker.publicIp}:3000/command`;
      console.log(`  Sending to ${worker.workerId} at ${workerUrl}`);

      try {
        const response = await fetch(workerUrl, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(command),
        });

        if (response.ok) {
          const result = await response.json();
          console.log(
            `  ✓ ${worker.workerId} responded: ${JSON.stringify(result)}`
          );
          return { ...result, success: true };
        }
        const errorText = await response.text();
        console.error(
          `  ✗ ${worker.workerId} HTTP ${response.status}: ${errorText}`
        );
        return {
          workerId: worker.workerId,
          success: false,
          error: `HTTP ${response.status}: ${errorText}`,
        };
      } catch (error) {
        console.error(`  ✗ ${worker.workerId} error: ${error}`);
        return {
          workerId: worker.workerId,
          success: false,
          error: String(error),
        };
      }
    });

    return Promise.all(promises);
  } catch (error) {
    console.error('Failed to broadcast command:', error);
    return [];
  }
}

async function checkWorkerHealth(coordinatorUrl: string): Promise<{
  healthy: string[];
  stale: string[];
  missing: string[];
}> {
  try {
    const response = await fetch(`${coordinatorUrl}/worker/status`);
    if (response.ok) {
      const data = await response.json();
      const workers = data.workers || [];
      const now = Date.now();
      const timeoutMs = 30000;

      const healthy: string[] = [];
      const stale: string[] = [];
      const missing: string[] = [];

      for (const worker of workers) {
        if (!worker.lastHeartbeat) {
          missing.push(worker.workerId);
        } else if (now - worker.lastHeartbeat > timeoutMs) {
          stale.push(worker.workerId);
        } else {
          healthy.push(worker.workerId);
        }
      }

      return { healthy, stale, missing };
    }
  } catch (error) {
    console.error('Failed to check worker health:', error);
  }
  return { healthy: [], stale: [], missing: [] };
}

async function waitForViteReady(
  publicIp: string,
  port: number,
  timeoutMs: number
): Promise<void> {
  const startTime = Date.now();
  const viteUrl = `http://${publicIp}:${port}`;

  while (Date.now() - startTime < timeoutMs) {
    try {
      const response = await fetch(viteUrl);
      if (response.ok) {
        console.log(`  ✓ Vite server ready at ${viteUrl}`);
        return;
      }
    } catch {
      // Continue waiting
    }
    await new Promise(resolve => setTimeout(resolve, 2000));
  }

  throw new Error(`Timeout waiting for Vite server at ${viteUrl}`);
}

function getPhaseEmoji(phase: TestPhase): string {
  switch (phase) {
    case 'provision':
      return '📦';
    case 'deploy':
      return '🚀';
    case 'warmup':
      return '🔥';
    case 'rampup':
      return '📈';
    case 'sustained':
      return '⚡';
    case 'stress':
      return '💥';
    case 'cooldown':
      return '❄️';
    case 'report':
      return '📊';
    case 'teardown':
      return '🧹';
    default:
      return '▶️';
  }
}

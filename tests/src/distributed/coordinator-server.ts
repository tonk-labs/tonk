#!/usr/bin/env node

import { WorkerCoordinator } from './worker-coordinator';
import type { TestScenario } from './types';

async function main() {
  const args = process.argv.slice(2);

  if (args.length < 1) {
    console.error('Usage: coordinator-server <port> [scenarioJson]');
    process.exit(1);
  }

  const port = parseInt(args[0], 10);

  let testScenario: TestScenario;

  if (args[1]) {
    try {
      const decoded = Buffer.from(args[1], 'base64').toString('utf-8');
      testScenario = JSON.parse(decoded);
    } catch (error) {
      console.error('Failed to parse scenario JSON:', error);
      process.exit(1);
    }
  } else {
    testScenario = {
      name: 'default',
      description: 'Default scenario',
      workerCount: 2,
      connectionsPerWorker: 50,
      workerInstanceType: 'c5.xlarge',
      relayHost: 'localhost',
      relayPort: 8080,
      useSpotInstances: true,
      phases: [],
    };
  }

  console.log('Starting coordinator server...');
  console.log(`Port: ${port}`);
  console.log(`Scenario: ${testScenario.name}`);

  const coordinator = new WorkerCoordinator(port, testScenario);

  const shutdown = async () => {
    console.log('\nShutting down coordinator...');
    await coordinator.stop();
    process.exit(0);
  };

  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);

  try {
    await coordinator.start();
    console.log('✓ Coordinator server is running');
  } catch (error) {
    console.error('Failed to start coordinator:', error);
    process.exit(1);
  }
}

main();

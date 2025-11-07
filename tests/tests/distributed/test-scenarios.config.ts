import type { TestScenario } from '../../src/distributed/types';

const RELAY_URL = process.env.RELAY_URL || 'https://relay.tonk.xyz';

export const scenarios: Record<string, TestScenario> = {
  baseline: {
    name: 'baseline',
    description: 'Baseline test with 100 connections to verify infrastructure',
    workerInstanceType: 'c5.xlarge',
    workerCount: 2,
    connectionsPerWorker: 50,
    relayUrl: RELAY_URL,
    useSpotInstances: true,
    maxCostPerHour: 1.0,
    phases: [
      {
        phase: 'warmup',
        durationMs: 2 * 60 * 1000,
        targetConnections: 10,
        operationsPerMinute: 30,
        description: 'Warm up with 10 connections',
      },
      {
        phase: 'rampup',
        durationMs: 5 * 60 * 1000,
        targetConnections: 100,
        operationsPerMinute: 30,
        description: 'Ramp up to 100 connections',
      },
      {
        phase: 'sustained',
        durationMs: 10 * 60 * 1000,
        targetConnections: 100,
        operationsPerMinute: 60,
        description: 'Sustained load at 100 connections',
      },
      {
        phase: 'cooldown',
        durationMs: 2 * 60 * 1000,
        targetConnections: 100,
        operationsPerMinute: 0,
        description: 'Cool down - no operations',
      },
    ],
  },

  moderate: {
    name: 'moderate',
    description: 'Moderate load test with 500 connections',
    workerInstanceType: 'c5.2xlarge',
    workerCount: 3,
    connectionsPerWorker: 167,
    relayUrl: RELAY_URL,
    useSpotInstances: true,
    maxCostPerHour: 2.0,
    phases: [
      {
        phase: 'warmup',
        durationMs: 3 * 60 * 1000,
        targetConnections: 50,
        operationsPerMinute: 30,
        description: 'Warm up with 50 connections',
      },
      {
        phase: 'rampup',
        durationMs: 10 * 60 * 1000,
        targetConnections: 500,
        operationsPerMinute: 30,
        description: 'Ramp up to 500 connections',
      },
      {
        phase: 'sustained',
        durationMs: 30 * 60 * 1000,
        targetConnections: 500,
        operationsPerMinute: 60,
        description: 'Sustained load at 500 connections',
      },
      {
        phase: 'cooldown',
        durationMs: 3 * 60 * 1000,
        targetConnections: 500,
        operationsPerMinute: 0,
        description: 'Cool down',
      },
    ],
  },

  heavy: {
    name: 'heavy',
    description: 'Heavy load test with 1000 connections',
    workerInstanceType: 'c5.2xlarge',
    workerCount: 5,
    connectionsPerWorker: 200,
    relayUrl: RELAY_URL,
    useSpotInstances: true,
    maxCostPerHour: 3.0,
    phases: [
      {
        phase: 'warmup',
        durationMs: 5 * 60 * 1000,
        targetConnections: 100,
        operationsPerMinute: 30,
        description: 'Warm up with 100 connections',
      },
      {
        phase: 'rampup',
        durationMs: 15 * 60 * 1000,
        targetConnections: 1000,
        operationsPerMinute: 30,
        description: 'Ramp up to 1000 connections',
      },
      {
        phase: 'sustained',
        durationMs: 60 * 60 * 1000,
        targetConnections: 1000,
        operationsPerMinute: 60,
        description: 'Sustained load at 1000 connections for 1 hour',
      },
      {
        phase: 'cooldown',
        durationMs: 5 * 60 * 1000,
        targetConnections: 1000,
        operationsPerMinute: 0,
        description: 'Cool down',
      },
    ],
  },

  stress: {
    name: 'stress',
    description: 'Stress test - push relay until it breaks',
    workerInstanceType: 'c5.2xlarge',
    workerCount: 50,
    connectionsPerWorker: 200,
    relayUrl: RELAY_URL,
    useSpotInstances: true,
    maxCostPerHour: 25.0,
    phases: [
      {
        phase: 'warmup',
        durationMs: 5 * 60 * 1000,
        targetConnections: 100,
        operationsPerMinute: 30,
        description: 'Warm up',
      },
      {
        phase: 'rampup',
        durationMs: 20 * 60 * 1000,
        targetConnections: 10000,
        operationsPerMinute: 60,
        description: 'Ramp up to 10000 connections',
      },
      {
        phase: 'stress',
        durationMs: 60 * 60 * 1000,
        targetConnections: 10000,
        operationsPerMinute: 120,
        description: 'Stress - increase operations until failure',
      },
      {
        phase: 'cooldown',
        durationMs: 5 * 60 * 1000,
        targetConnections: 10000,
        operationsPerMinute: 0,
        description: 'Cool down',
      },
    ],
  },
};

export function getScenario(name: string): TestScenario {
  const scenario = scenarios[name];
  if (!scenario) {
    throw new Error(
      `Unknown scenario: ${name}. Available: ${Object.keys(scenarios).join(', ')}`
    );
  }
  return scenario;
}

export function listScenarios(): string[] {
  return Object.keys(scenarios);
}

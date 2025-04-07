/**
 * Simple Workers Queue Manager
 *
 * A low-frills manager in pure Node.js that acts as a singleton.
 * It registers integrations with their schedule frequency and
 * creates interval timers to execute them.
 */

import path from 'path';
import {exec} from 'child_process';

/**
 * Integration configuration
 */
export interface IntegrationConfig {
  name: string;
  schedule: {
    frequency: string; // Format: '30s', '2m', '1hr', '200ms', etc.
  };
}

/**
 * Parse time string to milliseconds
 */
const parseTimeToMs = (timeStr: string): number => {
  const match = timeStr.match(/^(\d+)([a-z]+)$/i);
  if (!match) {
    throw new Error(
      `Invalid time format: ${timeStr}. Expected formats like '30s', '2m', '1hr', '200ms'.`,
    );
  }

  const [, valueStr, unit] = match;
  const value = parseInt(valueStr, 10);

  switch (unit.toLowerCase()) {
    case 'ms':
      return value;
    case 's':
      return value * 1000;
    case 'm':
      return value * 60 * 1000;
    case 'hr':
    case 'h':
      return value * 60 * 60 * 1000;
    case 'd':
      return value * 24 * 60 * 60 * 1000;
    default:
      throw new Error(`Unknown time unit: ${unit}`);
  }
};

/**
 * Worker Manager singleton
 */
export class WorkerManager {
  private static instance: WorkerManager;
  private integrations: Map<
    string,
    {config: IntegrationConfig; timerId: NodeJS.Timeout}
  >;

  private constructor() {
    this.integrations = new Map();
  }

  /**
   * Get singleton instance
   */
  public static getInstance(): WorkerManager {
    if (!WorkerManager.instance) {
      WorkerManager.instance = new WorkerManager();
    }
    return WorkerManager.instance;
  }

  /**
   * Register an integration with its schedule
   */
  public registerIntegration(config: IntegrationConfig): void {
    if (this.integrations.has(config.name)) {
      this.unregisterIntegration(config.name);
    }

    const frequencyMs = parseTimeToMs(config.schedule.frequency);

    const timerId = setInterval(() => {
      this.runIntegration(config.name);
    }, frequencyMs);

    this.integrations.set(config.name, {config, timerId});

    console.log(
      `Registered integration: ${config.name} (runs every ${config.schedule.frequency})`,
    );
  }

  /**
   * Unregister an integration
   */
  public unregisterIntegration(name: string): void {
    const integration = this.integrations.get(name);
    if (integration) {
      clearInterval(integration.timerId);
      this.integrations.delete(name);
      console.log(`Unregistered integration: ${name}`);
    }
  }

  /**
   * Run an integration
   */
  private runIntegration(name: string): void {
    console.log(`Running integration: ${name} at ${new Date().toISOString()}`);

    const integrationPath = path.join(process.cwd(), 'integrations', name);
    const command = `cd ${integrationPath} && node dist/index.js`;

    exec(command, (error, stdout, stderr) => {
      if (error) {
        console.error(`Error executing integration ${name}: ${error.message}`);
        return;
      }

      if (stderr) {
        console.error(`Integration ${name} stderr: ${stderr}`);
      }

      if (stdout) {
        console.log(`Integration ${name} output: ${stdout}`);
      }
    });
  }

  /**
   * Get all registered integrations
   */
  public getRegisteredIntegrations(): string[] {
    return Array.from(this.integrations.keys());
  }

  /**
   * Shutdown all integrations
   */
  public shutdown(): void {
    for (const name of this.getRegisteredIntegrations()) {
      this.unregisterIntegration(name);
    }
    console.log('Worker manager shutdown complete');
  }
}

// Export the singleton instance
export const workerManager = WorkerManager.getInstance();

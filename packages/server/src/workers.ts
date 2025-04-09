/**
 * Simple Workers Queue Manager
 *
 * A low-frills manager in pure Node.js that acts as a singleton.
 * It registers integrations with their schedule frequency and
 * creates interval timers to execute them.
 */

import path from 'path';
import {exec} from 'child_process';
import fs from 'fs';

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
export class Worker {
  private static instance: Worker;
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
  public static getInstance(): Worker {
    if (!Worker.instance) {
      Worker.instance = new Worker();
    }
    return Worker.instance;
  }

  /**
   * Register an integration with its schedule
   */
  public registerIntegration(config: IntegrationConfig): void {
    if (this.integrations.has(config.name)) {
      this.unregisterIntegration(config.name);
    }

    const frequencyMs = parseTimeToMs(config.schedule.frequency);

    this.runIntegration(config.name);

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

    const integrationPath = path.join(process.cwd(), 'integrations');

    // Sanitize the name for file paths, but keep original for require
    const sanitizedName = name.replace(/\//g, '-');

    // Create a temporary file that requires and executes the integration
    const tempFilePath = path.join(
      process.cwd(),
      'integrations',
      `.temp-${sanitizedName}-${Date.now()}.js`,
    );
    const tempFileContent = `
      (async () => {
        try {
          const integration = await import('${name}');
          if (typeof integration === 'function') {
            integration();
          } else if (integration.default && typeof integration.default === 'function') {
            integration.default();
          } else {
            console.error('Integration ${name} does not export a function');
          }
        } catch (error) {
          console.error(error);
          process.exit(1);
        }
      })();
    `;

    // Write the temp file
    fs.writeFileSync(tempFilePath, tempFileContent);

    // Execute the temp file
    const command = `cd ${integrationPath} && node --experimental-modules ${tempFilePath}`;

    exec(command, (error, stdout, stderr) => {
      // Clean up the temp file
      try {
        fs.unlinkSync(tempFilePath);
      } catch (cleanupError: unknown) {
        console.error(
          `Error cleaning up temp file: ${(cleanupError as Error).message}`,
        );
      }

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
export default Worker.getInstance();

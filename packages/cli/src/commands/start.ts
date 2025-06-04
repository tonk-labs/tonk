import {Command} from 'commander';
import chalk from 'chalk';
import fetch from 'node-fetch';
import {trackCommand, trackCommandError, trackCommandSuccess} from '../utils/analytics.js';

interface StartResult {
  id: string;
  bundleName: string;
  port: number;
  status: string;
}

export const startCommand = new Command('start')
  .description('Start a bundle server')
  .option('-u, --url <url>', 'URL of the Tonk server', 'http://localhost:7777')
  .option('-p, --port <port>', 'Port for the bundle server (optional)')
  .argument('<bundleName>', 'Name of the bundle to start')
  .action(async (bundleName, options) => {
    const startTime = Date.now();
    const serverUrl = options.url;

    try {
      trackCommand('start', {
        bundleName,
        serverUrl,
        port: options.port,
      });

      console.log(chalk.blue(`Starting bundle server for ${bundleName}...`));

      const requestBody: {bundleName: string; port?: number} = {
        bundleName,
      };

      // Add port if specified
      if (options.port) {
        requestBody.port = parseInt(options.port, 10);
      }

      const response = await fetch(`${serverUrl}/start`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(requestBody),
      });

      if (!response.ok) {
        const errorText = await response.text();
        throw new Error(`Server error: ${errorText}`);
      }

      const result = (await response.json()) as StartResult;

      console.log(chalk.green('Bundle server started successfully!'));
      console.log(chalk.green(`Server ID: ${result.id}`));
      console.log(chalk.green(`Running on port: ${result.port}`));
      console.log(chalk.green(`Status: ${result.status}`));
      console.log(chalk.blue(`Use 'tonk ps' to see all running servers`));

      const duration = Date.now() - startTime;
      trackCommandSuccess('start', duration, {
        bundleName,
        serverUrl,
        port: result.port,
        serverId: result.id,
      });
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('start', error as Error, duration, {
        bundleName,
        serverUrl,
      });

      console.error(
        chalk.red(
          `Error: ${error instanceof Error ? error.message : String(error)}`,
        ),
      );
      process.exit(1);
    }
  });

import {Command} from 'commander';
import chalk from 'chalk';
import fetch from 'node-fetch';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
} from '../utils/analytics.js';

interface StartResult {
  id: string;
  bundleName: string;
  port?: number;
  route?: string;
  status: string;
  url?: string;
}

export const startCommand = new Command('start')
  .description('Start a bundle on a route')
  .option('-u, --url <url>', 'URL of the Tonk server', 'http://localhost:7777')
  .option(
    '-r, --route <route>',
    'Route path for the bundle (defaults to /bundleName)',
  )
  .option(
    '-p, --port <port>',
    'Port for the bundle server (legacy option, prefer --route)',
  )
  .argument('<bundleName>', 'Name of the bundle to start')
  .action(async (bundleName, options) => {
    const startTime = Date.now();
    const serverUrl = options.url;

    try {
      trackCommand('start', {
        bundleName,
        serverUrl,
        route: options.route,
        port: options.port,
      });

      console.log(chalk.blue(`Starting bundle ${bundleName}...`));

      const requestBody: {bundleName: string; route?: string; port?: number} = {
        bundleName,
      };

      // Prefer route over port
      if (options.route) {
        requestBody.route = options.route;
      } else if (options.port) {
        requestBody.port = parseInt(options.port, 10);
      } else {
        // Default to route-based deployment
        requestBody.route = `/${bundleName}`;
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

      console.log(chalk.green('Bundle started successfully!'));
      console.log(chalk.green(`Server ID: ${result.id}`));

      if (result.route) {
        console.log(chalk.green(`Route: ${result.route}`));
        console.log(
          chalk.green(`URL: ${result.url || `${serverUrl}${result.route}`}`),
        );
      } else if (result.port) {
        // Fallback for legacy port-based deployments
        console.log(chalk.green(`Running on port: ${result.port}`));
      }

      console.log(chalk.green(`Status: ${result.status}`));
      console.log(chalk.blue(`Use 'tonk ps' to see all running bundles`));

      const duration = Date.now() - startTime;
      trackCommandSuccess('start', duration, {
        bundleName,
        serverUrl,
        route: result.route,
        port: result.port,
        serverId: result.id,
      });
      process.exit(0);
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

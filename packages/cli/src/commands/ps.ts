import { Command } from 'commander';
import chalk from 'chalk';
import fetch from 'node-fetch';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
  shutdownAnalytics,
} from '../utils/analytics.js';
import { getServerConfig } from '../config/environment.js';

interface ServerInfo {
  id: string;
  bundleName: string;
  port?: number;
  route?: string;
  status: string;
  startedAt?: string;
  url?: string;
}

export const psCommand = new Command('ps')
  .description('List running bundles')
  .option(
    '-u, --url <url>',
    'URL of the Tonk server',
    getServerConfig().defaultUrl
  )
  .action(async options => {
    const startTime = Date.now();
    const serverUrl = options.url;

    try {
      trackCommand('ps', { serverUrl });

      console.log(chalk.blue(`Fetching running servers from ${serverUrl}...`));

      const response = await fetch(`${serverUrl}/ps`);

      if (!response.ok) {
        const error = await response.text();
        throw new Error(`Server error: ${error}`);
      }

      const servers = (await response.json()) as ServerInfo[];

      if (servers.length === 0) {
        console.log(chalk.yellow('No servers currently running.'));

        const duration = Date.now() - startTime;
        trackCommandSuccess('ps', duration, {
          serverUrl,
          serverCount: 0,
        });
        await shutdownAnalytics();
      }

      console.log(chalk.green(`Running bundles (${servers.length}):`));

      // Format the output as a table
      console.log(
        `${chalk.bold('ID'.padEnd(36))} | ${chalk.bold(
          'Bundle'.padEnd(20)
        )} | ${chalk.bold('Route/Port'.padEnd(15))} | ${chalk.bold('Status')}`
      );
      console.log('-'.repeat(85));

      servers.forEach((server: ServerInfo) => {
        const routeOrPort =
          server.route || (server.port ? `:${server.port}` : 'N/A');
        console.log(
          `${server.id.padEnd(36)} | ${server.bundleName.padEnd(20)} | ${routeOrPort.padEnd(15)} | ${server.status}`
        );
      });

      const duration = Date.now() - startTime;
      trackCommandSuccess('ps', duration, {
        serverUrl,
        serverCount: servers.length,
        bundleNames: servers.map(s => s.bundleName),
      });
      await shutdownAnalytics();
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('ps', error as Error, duration, {
        serverUrl,
      });

      console.error(
        chalk.red(
          `Error: ${error instanceof Error ? error.message : String(error)}`
        )
      );
      await shutdownAnalytics();
      process.exitCode = 1;
    }
  });

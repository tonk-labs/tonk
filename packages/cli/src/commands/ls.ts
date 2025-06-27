import {Command} from 'commander';
import chalk from 'chalk';
import fetch from 'node-fetch';
import {trackCommand, trackCommandError, trackCommandSuccess} from '../utils/analytics.js';
import {getServerConfig} from '../config/environment.js';

export const lsCommand = new Command('ls')
  .description('List available bundles on the Tonk server')
  .option('-u, --url <url>', 'URL of the Tonk server', getServerConfig().defaultUrl)
  .action(async options => {
    const startTime = Date.now();
    const serverUrl = options.url;

    try {
      trackCommand('ls', {serverUrl});

      console.log(chalk.blue(`Fetching bundle list from ${serverUrl}...`));

      const response = await fetch(`${serverUrl}/ls`);

      if (!response.ok) {
        const error = await response.text();
        throw new Error(`Server error: ${error}`);
      }

      const bundles = (await response.json()) as string[];

      if (bundles.length === 0) {
        console.log(chalk.yellow('No bundles found on the server.'));
        
        const duration = Date.now() - startTime;
        trackCommandSuccess('ls', duration, {
          serverUrl,
          bundleCount: 0,
        });
        process.exit(0);
      }

      console.log(chalk.green(`Available bundles (${bundles.length}):`));
      bundles.forEach((bundle: string) => {
        console.log(`  - ${bundle}`);
      });

      const duration = Date.now() - startTime;
      trackCommandSuccess('ls', duration, {
        serverUrl,
        bundleCount: bundles.length,
        bundles,
      });
      process.exit(0);
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('ls', error as Error, duration, {
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

import {Command} from 'commander';
import chalk from 'chalk';
import fetch from 'node-fetch';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
} from '../utils/analytics.js';

export const deleteCommand = new Command('delete')
  .alias('rm')
  .description('Delete a bundle from the server')
  .option('-u, --url <url>', 'URL of the Tonk server', 'http://localhost:7777')
  .argument('<bundleName>', 'Name of the bundle to delete')
  .action(async (bundleName, options) => {
    const startTime = Date.now();
    const serverUrl = options.url;

    try {
      trackCommand('delete', {
        bundleName,
        serverUrl,
      });

      console.log(chalk.blue(`Deleting bundle: ${bundleName}...`));

      const response = await fetch(`${serverUrl}/delete`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({bundleName}),
      });

      if (!response.ok) {
        const errorText = await response.text();
        throw new Error(`Server error: ${errorText}`);
      }

      const result = (await response.json()) as {
        success: boolean;
        message: string;
      };

      if (result.success) {
        console.log(chalk.green('Bundle deleted successfully!'));
        console.log(chalk.green(result.message));

        const duration = Date.now() - startTime;
        trackCommandSuccess('delete', duration, {
          bundleName,
          serverUrl,
          success: true,
        });
        process.exit(0);
      } else {
        console.log(
          chalk.yellow('Bundle deletion command sent, but with warnings:'),
        );
        console.log(chalk.yellow(result.message));

        const duration = Date.now() - startTime;
        trackCommandSuccess('delete', duration, {
          bundleName,
          serverUrl,
          success: false,
          message: result.message,
        });
        process.exit(0);
      }
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('delete', error as Error, duration, {
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


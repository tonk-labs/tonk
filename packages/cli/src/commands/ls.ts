import {Command} from 'commander';
import chalk from 'chalk';
import fetch from 'node-fetch';

export const lsCommand = new Command('ls')
  .description('List available bundles on the Tonk server')
  .option('-u, --url <url>', 'URL of the Tonk server', 'http://localhost:7777')
  .action(async options => {
    const serverUrl = options.url;

    try {
      console.log(chalk.blue(`Fetching bundle list from ${serverUrl}...`));

      const response = await fetch(`${serverUrl}/ls`);

      if (!response.ok) {
        const error = await response.text();
        throw new Error(`Server error: ${error}`);
      }

      const bundles = (await response.json()) as string[];

      if (bundles.length === 0) {
        console.log(chalk.yellow('No bundles found on the server.'));
        return;
      }

      console.log(chalk.green(`Available bundles (${bundles.length}):`));
      bundles.forEach((bundle: string) => {
        console.log(`  - ${bundle}`);
      });
    } catch (error) {
      console.error(
        chalk.red(
          `Error: ${error instanceof Error ? error.message : String(error)}`,
        ),
      );
      process.exit(1);
    }
  });

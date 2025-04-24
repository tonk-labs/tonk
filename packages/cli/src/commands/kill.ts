import {Command} from 'commander';
import chalk from 'chalk';
import fetch from 'node-fetch';

export const killCommand = new Command('kill')
  .description('Stop a running bundle server')
  .option('-u, --url <url>', 'URL of the Tonk server', 'http://localhost:7777')
  .argument('<serverId>', 'ID of the server to stop')
  .action(async (serverId, options) => {
    const serverUrl = options.url;

    try {
      console.log(chalk.blue(`Stopping server with ID: ${serverId}...`));

      const response = await fetch(`${serverUrl}/kill`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({id: serverId}),
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
        console.log(chalk.green('Server stopped successfully!'));
        console.log(chalk.green(result.message));
      } else {
        console.log(
          chalk.yellow('Server stop command sent, but with warnings:'),
        );
        console.log(chalk.yellow(result.message));
      }
    } catch (error) {
      console.error(
        chalk.red(
          `Error: ${error instanceof Error ? error.message : String(error)}`,
        ),
      );
      process.exit(1);
    }
  });

import chalk from 'chalk';
import {Command} from 'commander';
import ngrok from 'ngrok';
import ora from 'ora';

export const exposeCommand = new Command('expose')
  .description('Expose a local server to the internet using ngrok')
  .option('-p, --port <port>', 'Port to expose (default: 8080)', '8080')
  .option(
    '-a, --auth <auth>',
    'Basic auth credentials in format username:password',
  )
  .option('-r, --region <region>', 'Region to use (default: us)', 'us')
  .option('--subdomain <subdomain>', 'Custom subdomain (requires ngrok Pro)')
  .action(async options => {
    const spinner = ora('Starting ngrok tunnel...').start();

    try {
      const url = await ngrok.connect({
        addr: options.port,
        basicAuth: options.auth,
        region: options.region,
        subdomain: options.subdomain,
      });

      spinner.succeed(
        `Tunnel established! Your local server on port ${chalk.cyan(options.port)} is now available at:\n${chalk.green(url)}`,
      );
      console.log(chalk.yellow('\nPress Ctrl+C to stop the tunnel'));

      // Keep the process running until interrupted
      process.on('SIGINT', async () => {
        const closeSpinner = ora('Closing ngrok tunnel...').start();
        await ngrok.disconnect();
        await ngrok.kill();
        closeSpinner.succeed('Ngrok tunnel closed');
        process.exit(0);
      });
    } catch (e) {
      const error = e as Error;
      spinner.fail(`Failed to establish tunnel: ${error.message}`);

      if (error.message.includes('failed to start ngrok')) {
        console.log(chalk.yellow('\nTips:'));
        console.log('- Make sure you have installed ngrok correctly');
        console.log(
          '- If using a custom subdomain or other Pro features, set your authtoken with:',
        );
        console.log(chalk.cyan('  npx ngrok authtoken YOUR_AUTH_TOKEN'));
      }

      process.exit(1);
    }
  });

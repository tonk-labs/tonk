import {Command} from 'commander';
import chalk from 'chalk';
import fetch from 'node-fetch';
import {spawn} from 'child_process';
import {trackCommand, trackCommandError, trackCommandSuccess} from '../utils/analytics.js';

interface BundleInfo {
  id: string;
  bundleName: string;
  port: number;
  status: string;
}

export const proxyCommand = new Command('proxy')
  .description('Create a reverse proxy to access a Tonk bundle')
  .option('-u, --url <url>', 'URL of the Tonk server', 'http://localhost:7777')
  .argument('<bundleName>', 'Name of the bundle to proxy')
  .action(async (bundleName, options) => {
    const startTime = Date.now();
    const serverUrl = options.url;

    try {
      trackCommand('proxy', {
        bundleName,
        serverUrl,
      });

      console.log(chalk.blue(`Setting up proxy for bundle ${bundleName}...`));
      
      // First, check if the bundle is running
      const response = await fetch(`${serverUrl}/ps`, {
        method: 'GET',
        headers: {
          'Content-Type': 'application/json',
        },
      });

      if (!response.ok) {
        const errorText = await response.text();
        throw new Error(`Server error: ${errorText}`);
      }

      const runningBundles = (await response.json()) as BundleInfo[];
      const targetBundle = runningBundles.find(bundle => bundle.bundleName === bundleName);

      if (!targetBundle) {
        throw new Error(`Bundle '${bundleName}' is not running. Start it first with 'tonk start ${bundleName}'`);
      }

      console.log(chalk.green(`Found running bundle: ${targetBundle.bundleName}`));
      console.log(chalk.green(`Bundle port: ${targetBundle.port}`));
      console.log(chalk.blue(`Creating SSH tunnel with Pinggy...`));

      const duration = Date.now() - startTime;
      trackCommandSuccess('proxy', duration, {
        bundleName,
        serverUrl,
        bundlePort: targetBundle.port,
        bundleId: targetBundle.id,
      });

      // Create SSH tunnel using Pinggy
      const sshProcess = spawn('ssh', [
        '-p', '443',
        '-R0:localhost:' + targetBundle.port,
        'qr@free.pinggy.io'
      ], {
        stdio: 'inherit' // This will show the QR code and URLs directly in the terminal
      });

      // Handle process exit
      sshProcess.on('close', (code) => {
        if (code !== 0) {
          console.log(chalk.yellow(`SSH tunnel closed with code ${code}`));
        }
        console.log(chalk.green('Proxy connection terminated.'));
      });

      // Handle errors
      sshProcess.on('error', (err) => {
        console.error(chalk.red(`Error creating SSH tunnel: ${err.message}`));
        process.exit(1);
      });

      console.log(chalk.blue(`Press Ctrl+C to stop the proxy`));

      // Handle SIGINT (Ctrl+C)
      process.on('SIGINT', () => {
        console.log(chalk.yellow('\nShutting down proxy...'));
        sshProcess.kill();
      });

    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('proxy', error as Error, duration, {
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

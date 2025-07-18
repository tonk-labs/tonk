import {Command} from 'commander';
import displayTonkAnimation from './hello/index.js';
import chalk from 'chalk';
import envPaths from 'env-paths';
import {exec} from 'child_process';
import {promisify} from 'util';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
  shutdownAnalytics,
} from '../utils/analytics.js';

const execAsync = promisify(exec);

export const helloCommand = new Command('hello')
  .description('Say hello to start and launch the tonk daemon')
  .action(async () => {
    const startTime = Date.now();

    try {
      trackCommand('hello', {});

      // Check if pm2 is installed
      let pm2Exists = false;
      try {
        await execAsync('pm2 --version');
        pm2Exists = true;
      } catch (error) {
        // pm2 is not installed or not in PATH
        console.log(
          chalk.blue('PM2 is required to run Tonk. Installing now...'),
        );
        await execAsync('npm install -g pm2');
        pm2Exists = true;
        console.log(chalk.green('PM2 installed successfully.'));
      }

      if (pm2Exists) {
        console.log(chalk.blue('Starting Tonk daemon...'));
        try {
          // Check if tonk process is already running in PM2
          const {stdout} = await execAsync('pm2 list');

          if (stdout.includes('tonkserver')) {
            console.log(
              chalk.yellow('Tonk daemon is already running. Restarting...'),
            );
            await execAsync('pm2 restart tonkserver');
          } else {
            // Start the tonk daemon with PM2
            const {stdout: whichTonk} = await execAsync('which tonk');
            const tonkPath = whichTonk.trim();
            
            // Check if tonk executable is a bash script or Node.js file
            const {stdout: fileHead} = await execAsync(`head -1 ${tonkPath}`);
            const isBashScript = fileHead.includes('#!/bin/bash');
            
            //development is for when running locally, we don't use an NGINX instance in local development mode
            if (isBashScript) {
              await execAsync(
                `NODE_ENV=development pm2 start bash --name tonkserver -- ${tonkPath} -d`,
              );
            } else {
              await execAsync(
                `NODE_ENV=development pm2 start ${tonkPath} --name tonkserver -- -d`,
              );
            }
          }

          console.log(chalk.green('Tonk daemon started successfully!'));

          const paths = envPaths('tonk', {suffix: ''});
          const tonkHome = paths.data;

          // Pretty print the Tonk home location
          console.log(chalk.cyan('🏠 Tonk Home:'), chalk.bold.green(tonkHome));

          // Display the welcome animation
          displayTonkAnimation();

          const duration = Date.now() - startTime;
          trackCommandSuccess('hello', duration, {
            pm2AlreadyInstalled: pm2Exists,
            tonkAlreadyRunning: stdout.includes('tonkserver'),
          });
          await shutdownAnalytics();
        } catch (error) {
          const duration = Date.now() - startTime;
          trackCommandError('hello', error as Error, duration);
          console.error(chalk.red('Failed to start tonk daemon:'), error);
          await shutdownAnalytics();
          process.exitCode = 1;
        }
      }
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('hello', error as Error, duration);
      console.error(chalk.red('An error occurred:'), error);
      await shutdownAnalytics();
      process.exitCode = 1;
    }
  });

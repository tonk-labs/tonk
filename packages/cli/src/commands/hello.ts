import {Command} from 'commander';
import displayTonkAnimation from './hello/index.js';
import chalk from 'chalk';
import inquirer from 'inquirer';
import envPaths from 'env-paths';
import {exec} from 'child_process';
import {promisify} from 'util';
import {trackCommand, trackCommandError, trackCommandSuccess} from '../utils/analytics.js';

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
        console.log(chalk.yellow('PM2 is required to run the tonk daemon.'));

        const {installPm2} = await inquirer.prompt([
          {
            type: 'confirm',
            name: 'installPm2',
            message: 'Would you like to install PM2?',
            default: true,
          },
        ]);

        if (installPm2) {
          console.log(chalk.blue('Installing PM2...'));
          await execAsync('npm install -g pm2');
          pm2Exists = true;
          console.log(chalk.green('PM2 installed successfully.'));
        } else {
          console.log(
            chalk.red('PM2 is required to run the tonk daemon. Exiting.'),
          );
          process.exit(1);
        }
      }

      if (pm2Exists) {
        console.log(chalk.blue('Starting tonk daemon with PM2...'));
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
            await execAsync('pm2 start tonk --name tonkserver -- -d');
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
        } catch (error) {
          const duration = Date.now() - startTime;
          trackCommandError('hello', error as Error, duration);
          console.error(chalk.red('Failed to start tonk daemon:'), error);
          process.exit(1);
        }
      }
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('hello', error as Error, duration);
      console.error(chalk.red('An error occurred:'), error);
      process.exit(1);
    }
  });

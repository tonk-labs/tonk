import {Command} from 'commander';
import {createCommand} from './commands/create.js';
import {deployCommand} from './commands/deploy.js';
import {serverCommand} from './commands/server.js';
import {helloCommand} from './commands/hello.js';
import {lsCommand} from './commands/ls.js';
import {psCommand} from './commands/ps.js';
import {startCommand} from './commands/start.js';
import {killCommand} from './commands/kill.js';
import {deleteCommand} from './commands/delete.js';
import {pushCommand} from './commands/push.js';
import {proxyCommand} from './commands/proxy.js';
import {workerCommand} from './commands/worker/index.js';
import {createServer} from '@tonk/server';
import chalk from 'chalk';
import envPaths from 'env-paths';
import fs from 'node:fs';
import path from 'node:path';
import pkg from '../package.json' with {type: 'json'};
import {
  shutdownAnalytics,
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
} from './utils/analytics.js';
import { authCommand } from './commands/auth.js';

const program = new Command();
// Main program setup
program
  .name('tonk')
  .description('The tonk cli helps you to manage your tonk stack and apps.')
  .version(pkg.version)
  .option('-d', 'Run the Tonk daemon')
  .on('--help', () => {
    console.log('\nWork in progress!');
  });

  program.addCommand(helloCommand);
  program.addCommand(authCommand);
program.addCommand(createCommand);
program.addCommand(deployCommand);
program.addCommand(serverCommand);
// Add bundle management commands
program.addCommand(pushCommand);
program.addCommand(startCommand);
program.addCommand(psCommand);
program.addCommand(lsCommand);
program.addCommand(killCommand);
program.addCommand(deleteCommand);
program.addCommand(proxyCommand);
program.addCommand(workerCommand);

const startServer = async () => {
  const startTime = Date.now();

  try {
    trackCommand('daemon', {});

    // Create paths for Tonk daemon home
    const paths = envPaths('tonk', {suffix: ''});
    const tonkHome = paths.data;
    const bundlesPath = path.join(tonkHome, 'bundles');
    const storesPath = path.join(tonkHome, 'stores');

    // Pretty print the Tonk home location
    console.log(chalk.cyan('🏠 Tonk Home:'), chalk.bold.green(tonkHome));

    // Ensure directories exist
    for (const dir of [tonkHome, bundlesPath, storesPath]) {
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, {recursive: true});
      }
    }

    console.log(chalk.blue('Starting TonkServer daemon...'));
    console.log(chalk.blue(`Using Tonk home directory: ${tonkHome}`));

    // Create server configuration matching ServerOptions interface
    const serverConfig = {
      bundlesPath,
      storesPath,
    };

    // Start the server
    const server = await createServer(serverConfig);

    const duration = Date.now() - startTime;
    trackCommandSuccess('daemon', duration);

    // Handle process termination
    const cleanup = async () => {
      console.log(chalk.yellow('\nShutting down server...'));
      try {
        await server.stop();
        await shutdownAnalytics();
        console.log(chalk.green('Server shutdown complete.'));
        // Force exit after brief delay to ensure any pending operations complete
        setTimeout(() => process.exit(0), 500);
      } catch (err) {
        console.error(chalk.red('Error during shutdown:'), err);
        // Force exit even on error
        process.exit(1);
      }
    };

    process.on('SIGINT', cleanup);
    process.on('SIGTERM', cleanup);
  } catch (error) {
    const duration = Date.now() - startTime;
    trackCommandError('daemon', error as Error, duration);
    console.error(chalk.red('Failed to start TonkServer daemon:'), error);
    process.exit(1);
  }
};

if (process.argv.length <= 3 && process.argv[2] === '-d') {
  startServer();
} else {
  program.parse(process.argv);
}

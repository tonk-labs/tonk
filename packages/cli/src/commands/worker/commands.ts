import chalk from 'chalk';
import fs from 'node:fs';
import path from 'node:path';
import Table from 'cli-table3';
import {Command} from 'commander';
import {promisify} from 'node:util';
import {exec} from 'node:child_process';
import {TonkWorkerManager} from '../../lib/workerManager.js';
import {displayWorkerDetails} from './utils/display.js';
import {findWorkerRoot, findAvailablePort} from './utils/finder.js';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
  shutdownAnalytics,
} from '../../utils/analytics.js';
import {
  readPackageJson,
  readWorkerConfigJs,
  updateWorkerConfig,
  promptForMissingOptions,
  generatePackageJsonContent,
  generateWorkerConfigJsContent,
} from './utils/config.js';

/**
 * Register the inspect command
 */
export function registerInspectCommand(workerCommand: Command): void {
  workerCommand
    .command('inspect <nameOrId>')
    .description('Inspect a specific worker')
    .option('-s, --start', 'Start the worker')
    .option('-S, --stop', 'Stop the worker')
    .option('-c, --config <path>', 'Path to worker configuration file')
    .option('-p, --ping', 'Ping the worker to check its status')
    .action(async (nameOrId, options) => {
      const startTime = Date.now();

      try {
        trackCommand('worker-inspect', {
          nameOrId,
          start: options.start,
          stop: options.stop,
          config: !!options.config,
          ping: options.ping,
        });

        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.findByNameOrId(nameOrId);

        if (!worker) {
          console.error(
            chalk.red(`Worker with name or ID '${nameOrId}' not found.`),
          );

          const duration = Date.now() - startTime;
          trackCommandError(
            'worker-inspect',
            new Error('Worker not found'),
            duration,
            {
              nameOrId,
            },
          );
          await shutdownAnalytics();
          process.exit(1);
        }

        // If no options provided, show worker details
        if (
          !options.start &&
          !options.stop &&
          !options.config &&
          !options.ping
        ) {
          displayWorkerDetails(worker);

          const duration = Date.now() - startTime;
          trackCommandSuccess('worker-inspect', duration, {
            workerId: worker.id,
            workerName: worker.name,
            action: 'display',
          });
          await shutdownAnalytics();
          process.exit(0);
        }

        // Handle ping option
        if (options.ping) {
          console.log(
            chalk.blue(
              `Pinging worker '${worker.name}' at ${worker.endpoint}...`,
            ),
          );

          // Check worker health
          const isHealthy = await workerManager.checkHealth(worker.id);

          if (isHealthy) {
            console.log(chalk.green(`Worker '${worker.name}' is active!`));
          } else {
            console.log(
              chalk.red(`Worker '${worker.name}' is not responding.`),
            );
          }

          const duration = Date.now() - startTime;
          trackCommandSuccess('worker-inspect', duration, {
            workerId: worker.id,
            workerName: worker.name,
            action: 'ping',
            isHealthy,
          });
          await shutdownAnalytics();
          process.exit(0);
        }

        // Handle start option
        if (options.start) {
          console.log(chalk.blue(`Starting worker '${worker.name}'...`));
          await workerManager.start(worker.id);
          console.log(
            chalk.green(`Worker '${worker.name}' started successfully!`),
          );

          const duration = Date.now() - startTime;
          trackCommandSuccess('worker-inspect', duration, {
            workerId: worker.id,
            workerName: worker.name,
            action: 'start',
          });
          await shutdownAnalytics();
          process.exit(0);
        }

        // Handle stop option
        if (options.stop) {
          console.log(chalk.blue(`Stopping worker '${worker.name}'...`));
          await workerManager.stop(worker.id);
          console.log(
            chalk.green(`Worker '${worker.name}' stopped successfully!`),
          );

          const duration = Date.now() - startTime;
          trackCommandSuccess('worker-inspect', duration, {
            workerId: worker.id,
            workerName: worker.name,
            action: 'stop',
          });
          await shutdownAnalytics();
          process.exit(0);
        }

        // Handle config option
        if (options.config) {
          await updateWorkerConfig(workerManager, worker.id, options.config);

          const duration = Date.now() - startTime;
          trackCommandSuccess('worker-inspect', duration, {
            workerId: worker.id,
            workerName: worker.name,
            action: 'config',
            configPath: options.config,
          });
          await shutdownAnalytics();
          process.exit(0);
        }
      } catch (error) {
        const duration = Date.now() - startTime;
        trackCommandError('worker-inspect', error as Error, duration, {
          nameOrId,
          options,
        });
        console.error(chalk.red('Failed to manage worker:'), error);
        await shutdownAnalytics();
        process.exit(1);
      }
    });
}

/**
 * Register the list command
 */
export function registerListCommand(workerCommand: Command): void {
  workerCommand
    .command('ls')
    .description('List all registered workers')
    .action(async () => {
      const startTime = Date.now();

      try {
        trackCommand('worker-ls', {});

        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get all workers
        const workers = await workerManager.list();

        if (workers.length === 0) {
          console.log(chalk.yellow('No workers registered yet.'));
          console.log(
            chalk.blue(
              `Use '${chalk.bold('tonk worker register')}' to register a worker.`,
            ),
          );

          const duration = Date.now() - startTime;
          trackCommandSuccess('worker-ls', duration, {
            workerCount: 0,
          });
          await shutdownAnalytics();
          process.exit(0);
        }

        // Create a table for display
        const table = new Table({
          head: [
            chalk.cyan('ID'),
            chalk.cyan('Name'),
            chalk.cyan('Endpoint'),
            chalk.cyan('Protocol'),
            chalk.cyan('Status'),
            chalk.cyan('Last Seen'),
          ],
          colWidths: [24, 20, 30, 10, 10, 20],
        });

        // Add workers to table
        workers.forEach(worker => {
          const lastSeen = worker.status.lastSeen
            ? new Date(worker.status.lastSeen).toLocaleString()
            : 'Never';

          table.push([
            worker.id,
            worker.name,
            worker.endpoint,
            worker.protocol,
            worker.status.active
              ? chalk.green('Active')
              : chalk.yellow('Inactive'),
            lastSeen,
          ]);
        });

        console.log(chalk.bold(`Registered Workers (${workers.length}):`));
        console.log(table.toString());
        console.log(
          chalk.blue(
            `\nUse '${chalk.bold('tonk worker inspect <name or id>')}' to view details of a specific worker.`,
          ),
        );

        const duration = Date.now() - startTime;
        trackCommandSuccess('worker-ls', duration, {
          workerCount: workers.length,
          activeWorkers: workers.filter(w => w.status.active).length,
        });
        await shutdownAnalytics();
        process.exit(0);
      } catch (error) {
        const duration = Date.now() - startTime;
        trackCommandError('worker-ls', error as Error, duration);
        console.error(chalk.red('Failed to list workers:'), error);
        await shutdownAnalytics();
        process.exit(1);
      }
    });
}

/**
 * Register the remove command
 */
export function registerRemoveCommand(workerCommand: Command): void {
  workerCommand
    .command('rm <nameOrId>')
    .description('Remove a registered worker')
    .action(async nameOrId => {
      const startTime = Date.now();

      try {
        trackCommand('worker-rm', {
          nameOrId,
        });

        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.findByNameOrId(nameOrId);

        if (!worker) {
          console.error(
            chalk.red(`Worker with name or ID '${nameOrId}' not found.`),
          );

          const duration = Date.now() - startTime;
          trackCommandError(
            'worker-rm',
            new Error('Worker not found'),
            duration,
            {
              nameOrId,
            },
          );
          await shutdownAnalytics();
          process.exit(1);
        }

        // Remove worker
        await workerManager.remove(worker.id);
        console.log(
          chalk.green(
            `Worker '${worker.name}' (${worker.id}) removed successfully.`,
          ),
        );

        const duration = Date.now() - startTime;
        trackCommandSuccess('worker-rm', duration, {
          workerId: worker.id,
          workerName: worker.name,
        });
        await shutdownAnalytics();
        process.exit(0);
      } catch (error) {
        const duration = Date.now() - startTime;
        trackCommandError('worker-rm', error as Error, duration, {
          nameOrId,
        });
        console.error(chalk.red('Failed to remove worker:'), error);
        await shutdownAnalytics();
        process.exit(1);
      }
    });
}

/**
 * Register the ping command
 */
export function registerPingCommand(workerCommand: Command): void {
  workerCommand
    .command('ping <nameOrId>')
    .description('Ping a worker to check its status')
    .action(async nameOrId => {
      const startTime = Date.now();

      try {
        trackCommand('worker-ping', {
          nameOrId,
        });

        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.findByNameOrId(nameOrId);

        if (!worker) {
          console.error(
            chalk.red(`Worker with name or ID '${nameOrId}' not found.`),
          );

          const duration = Date.now() - startTime;
          trackCommandError(
            'worker-ping',
            new Error('Worker not found'),
            duration,
            {
              nameOrId,
            },
          );
          await shutdownAnalytics();
          process.exit(1);
        }

        console.log(
          chalk.blue(
            `Pinging worker '${worker.name}' at ${worker.endpoint}...`,
          ),
        );

        // Check worker health
        const isHealthy = await workerManager.checkHealth(worker.id);

        if (isHealthy) {
          console.log(chalk.green(`Worker '${worker.name}' is active!`));
        } else {
          console.log(chalk.red(`Worker '${worker.name}' is not responding.`));
        }

        const duration = Date.now() - startTime;
        trackCommandSuccess('worker-ping', duration, {
          workerId: worker.id,
          workerName: worker.name,
          isHealthy,
        });
        await shutdownAnalytics();
        process.exit(0);
      } catch (error) {
        const duration = Date.now() - startTime;
        trackCommandError('worker-ping', error as Error, duration, {
          nameOrId,
        });
        console.error(chalk.red('Failed to ping worker:'), error);
        await shutdownAnalytics();
        process.exit(1);
      }
    });
}

/**
 * Register the start command
 */
export function registerStartCommand(workerCommand: Command): void {
  workerCommand
    .command('start <nameOrId>')
    .description('Start a worker')
    .action(async nameOrId => {
      const startTime = Date.now();

      try {
        trackCommand('worker-start', {
          nameOrId,
        });

        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.findByNameOrId(nameOrId);

        if (!worker) {
          console.error(
            chalk.red(`Worker with name or ID '${nameOrId}' not found.`),
          );

          const duration = Date.now() - startTime;
          trackCommandError(
            'worker-start',
            new Error('Worker not found'),
            duration,
            {
              nameOrId,
            },
          );
          await shutdownAnalytics();
          process.exit(1);
        }

        console.log(chalk.blue(`Starting worker '${worker.name}'...`));
        await workerManager.start(worker.id);
        console.log(
          chalk.green(`Worker '${worker.name}' started successfully!`),
        );

        const duration = Date.now() - startTime;
        trackCommandSuccess('worker-start', duration, {
          workerId: worker.id,
          workerName: worker.name,
        });
        await shutdownAnalytics();
        process.exit(0);
      } catch (error) {
        const duration = Date.now() - startTime;
        trackCommandError('worker-start', error as Error, duration, {
          nameOrId,
        });
        console.error(chalk.red('Failed to start worker:'), error);
        await shutdownAnalytics();
        process.exit(1);
      }
    });
}

/**
 * Register the stop command
 */
export function registerStopCommand(workerCommand: Command): void {
  workerCommand
    .command('stop <nameOrId>')
    .description('Stop a worker')
    .action(async nameOrId => {
      const startTime = Date.now();

      try {
        trackCommand('worker-stop', {
          nameOrId,
        });

        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.findByNameOrId(nameOrId);

        if (!worker) {
          console.error(
            chalk.red(`Worker with name or ID '${nameOrId}' not found.`),
          );

          const duration = Date.now() - startTime;
          trackCommandError(
            'worker-stop',
            new Error('Worker not found'),
            duration,
            {
              nameOrId,
            },
          );
          await shutdownAnalytics();
          process.exit(1);
        }

        console.log(chalk.blue(`Stopping worker '${worker.name}'...`));
        await workerManager.stop(worker.id);
        console.log(
          chalk.green(`Worker '${worker.name}' stopped successfully!`),
        );

        const duration = Date.now() - startTime;
        trackCommandSuccess('worker-stop', duration, {
          workerId: worker.id,
          workerName: worker.name,
        });
        await shutdownAnalytics();
        process.exit(0);
      } catch (error) {
        const duration = Date.now() - startTime;
        trackCommandError('worker-stop', error as Error, duration, {
          nameOrId,
        });
        console.error(chalk.red('Failed to stop worker:'), error);
        await shutdownAnalytics();
        process.exit(1);
      }
    });
}

/**
 * Register the logs command
 */
export function registerLogsCommand(workerCommand: Command): void {
  workerCommand
    .command('logs <nameOrId>')
    .description('View logs for a worker')
    .option('-f, --follow', 'Follow log output')
    .option('-l, --lines <n>', 'Number of lines to show', '100')
    .option('-e, --error', 'Show only error logs')
    .option('-o, --out', 'Show only standard output logs')
    .action(async (nameOrId, options) => {
      const startTime = Date.now();

      try {
        trackCommand('worker-logs', {
          nameOrId,
          follow: options.follow,
          lines: options.lines,
          error: options.error,
          out: options.out,
        });

        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.findByNameOrId(nameOrId);

        if (!worker) {
          console.error(
            chalk.red(`Worker with name or ID '${nameOrId}' not found.`),
          );

          const duration = Date.now() - startTime;
          trackCommandError(
            'worker-logs',
            new Error('Worker not found'),
            duration,
            {
              nameOrId,
            },
          );
          await shutdownAnalytics();
          process.exit(1);
        }

        console.log(chalk.blue(`Fetching logs for worker '${worker.name}'...`));

        const {spawn} = await import('node:child_process');

        try {
          // Build the PM2 logs command arguments
          const args = ['logs', worker.id];

          // Add options
          if (options.lines) {
            args.push('--lines', options.lines);
          }

          if (options.error) {
            args.push('--err');
          } else if (options.out) {
            args.push('--out');
          }

          // Always use --raw to get cleaner output
          args.push('--raw');

          // For non-follow mode, we'll use --lines and then kill the process after a short delay
          if (!options.follow) {
            console.log(
              chalk.blue(
                `Showing last ${options.lines || '100'} lines of logs...`,
              ),
            );

            const child = spawn('pm2', args, {
              stdio: 'inherit',
            });

            // Kill the process after a short delay to get just the initial output
            setTimeout(() => {
              child.kill('SIGINT');
            }, 1000);

            // Wait for the child process to exit
            await new Promise(resolve => {
              child.on('exit', resolve);
            });

            console.log(
              chalk.blue('\nUse -f or --follow to stream logs in real-time'),
            );
          } else {
            // For follow mode
            console.log(
              chalk.blue('Streaming logs in real-time. Press Ctrl+C to exit.'),
            );

            const child = spawn('pm2', args, {
              stdio: 'inherit',
            });

            // Handle process exit
            process.on('SIGINT', async () => {
              child.kill();
              await shutdownAnalytics();
              process.exit(0);
            });

            // Wait for the child process to exit
            await new Promise(resolve => {
              child.on('exit', resolve);
            });
          }

          const duration = Date.now() - startTime;
          trackCommandSuccess('worker-logs', duration, {
            workerId: worker.id,
            workerName: worker.name,
            follow: options.follow,
            lines: options.lines,
            logType: options.error ? 'error' : options.out ? 'out' : 'all',
          });
          await shutdownAnalytics();
          process.exit(0);
        } catch (error) {
          const duration = Date.now() - startTime;
          trackCommandError('worker-logs', error as Error, duration, {
            workerId: worker.id,
            workerName: worker.name,
            options,
          });
          console.error(chalk.red('Failed to fetch logs:'), error);
          await shutdownAnalytics();
          process.exit(1);
        }
      } catch (error) {
        const duration = Date.now() - startTime;
        trackCommandError('worker-logs', error as Error, duration, {
          nameOrId,
          options,
        });
        console.error(chalk.red('Failed to fetch worker logs:'), error);
        await shutdownAnalytics();
        process.exit(1);
      }
    });
}

/**
 * Register the register command
 */
export function registerRegisterCommand(workerCommand: Command): void {
  workerCommand
    .command('register')
    .description('Register a worker with Tonk')
    .argument(
      '[dir]',
      'Path to worker directory (defaults to current directory)',
    )
    .option('-n, --name <n>', 'Name of the worker')
    .option('-e, --endpoint <endpoint>', 'Endpoint URL of the worker')
    .option('-p, --port <port>', 'Port number for the worker')
    .option('-d, --description <description>', 'Description of the worker')
    .action(async (dir = '.', options) => {
      const startTime = Date.now();

      try {
        trackCommand('worker-register', {
          hasDir: !!dir && dir !== '.',
          hasName: !!options.name,
          hasEndpoint: !!options.endpoint,
          hasPort: !!options.port,
          hasDescription: !!options.description,
        });

        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // If options are provided, use them directly
        if (options.name && options.endpoint) {
          // Register worker using provided options
          const worker = await workerManager.register({
            name: options.name,
            description: options.description || `Worker at ${options.endpoint}`,
            endpoint: options.endpoint,
            protocol: 'http',
            env: options.port ? [`WORKER_PORT=${options.port}`] : [],
          });

          console.log(
            chalk.green(`Worker '${worker.name}' registered successfully!`),
          );
          console.log(chalk.cyan('Worker ID:'), chalk.bold(worker.id));
          console.log(chalk.cyan('Endpoint:'), chalk.bold(worker.endpoint));
          console.log(chalk.cyan('Protocol:'), chalk.bold(worker.protocol));
          console.log(
            chalk.cyan('Status:'),
            worker.status.active
              ? chalk.green('Active')
              : chalk.yellow('Inactive'),
          );

          if (Object.keys(worker.env).length > 0) {
            console.log(chalk.cyan('Environment Variables:'));
            Object.entries(worker.env).forEach(([key, value]) => {
              console.log(`  ${key}=${value}`);
            });
          }

          const duration = Date.now() - startTime;
          trackCommandSuccess('worker-register', duration, {
            workerId: worker.id,
            workerName: worker.name,
            registrationType: 'direct',
            hasCustomPort: !!options.port,
          });
          await shutdownAnalytics();
          process.exit(0);
        }

        // Look for package.json and worker.config.js files
        const workerDir = await findWorkerRoot(dir);

        if (!workerDir) {
          console.error(
            chalk.red(
              `No package.json or worker.config.js found in ${path.resolve(dir)} or its parent directories.`,
            ),
          );
          console.log(
            chalk.yellow(
              'Make sure you are in a valid worker directory or specify the path to one, or provide --name and --endpoint options.',
            ),
          );

          const duration = Date.now() - startTime;
          trackCommandError(
            'worker-register',
            new Error('Worker directory not found'),
            duration,
            {
              dir: path.resolve(dir),
            },
          );
          await shutdownAnalytics();
          process.exit(1);
        }

        console.log(chalk.blue(`Found worker directory at: ${workerDir}`));

        // Read package.json for worker metadata
        const packageJsonPath = path.join(workerDir, 'package.json');
        const packageJson = await readPackageJson(packageJsonPath);

        if (!packageJson) {
          console.error(
            chalk.red(
              `Failed to parse package.json file at ${packageJsonPath}`,
            ),
          );

          const duration = Date.now() - startTime;
          trackCommandError(
            'worker-register',
            new Error('Failed to parse package.json'),
            duration,
            {
              packageJsonPath,
            },
          );
          await shutdownAnalytics();
          process.exit(1);
        }

        // Read worker.config.js for configuration
        const workerConfigPath = path.join(workerDir, 'worker.config.js');
        const workerConfig = await readWorkerConfigJs(workerConfigPath);

        if (!workerConfig) {
          console.error(
            chalk.red(
              `Failed to parse worker.config.js file at ${workerConfigPath}`,
            ),
          );

          const duration = Date.now() - startTime;
          trackCommandError(
            'worker-register',
            new Error('Failed to parse worker.config.js'),
            duration,
            {
              workerConfigPath,
            },
          );
          await shutdownAnalytics();
          process.exit(1);
        }

        // Determine port from config or use default
        const port = options.port || workerConfig.runtime?.port || 5555;

        // Create worker registration options
        const registrationOptions = {
          name: packageJson.name,
          description: packageJson.description || 'Tonk worker',
          endpoint: options.endpoint || `http://localhost:${port}/tonk`,
          protocol: 'http',
          env: [`WORKER_PORT=${port}`],
          config: {
            ...workerConfig,
            version: packageJson.version,
          },
        };

        // Register worker
        const worker = await workerManager.register(registrationOptions);

        console.log(
          chalk.green(`Worker '${worker.name}' registered successfully!`),
        );
        console.log(chalk.cyan('Worker ID:'), chalk.bold(worker.id));
        console.log(chalk.cyan('Endpoint:'), chalk.bold(worker.endpoint));
        console.log(chalk.cyan('Protocol:'), chalk.bold(worker.protocol));
        console.log(
          chalk.cyan('Status:'),
          worker.status.active
            ? chalk.green('Active')
            : chalk.yellow('Inactive'),
        );

        if (Object.keys(worker.env).length > 0) {
          console.log(chalk.cyan('Environment Variables:'));
          Object.entries(worker.env).forEach(([key, value]) => {
            console.log(`  ${key}=${value}`);
          });
        }

        const duration = Date.now() - startTime;
        trackCommandSuccess('worker-register', duration, {
          workerId: worker.id,
          workerName: worker.name,
          registrationType: 'config-based',
          workerDir,
          port,
          hasCustomEndpoint: !!options.endpoint,
        });
        await shutdownAnalytics();
        process.exit(0);
      } catch (error) {
        const duration = Date.now() - startTime;
        trackCommandError('worker-register', error as Error, duration, {
          dir,
          options,
        });
        console.error(chalk.red('Failed to register worker:'), error);
        await shutdownAnalytics();
        process.exit(1);
      }
    });
}

/**
 * Register the install command
 */
export function registerInstallCommand(workerCommand: Command): void {
  workerCommand
    .command('install <package>')
    .description('Install and start a worker from npm')
    .option(
      '-p, --port <port>',
      'Specify a port for the worker (default: auto-detect)',
    )
    .option(
      '-n, --name <n>',
      'Custom name for the worker (default: npm package name)',
    )
    .action(async (packageName, options) => {
      const startTime = Date.now();

      try {
        trackCommand('worker-install', {
          packageName,
          hasCustomPort: !!options.port,
          hasCustomName: !!options.name,
        });

        console.log(
          chalk.blue(`Installing worker from npm package: ${packageName}...`),
        );

        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Execute npm install
        const execAsync = promisify(exec);

        try {
          console.log(chalk.blue('Installing package...'));
          await execAsync(`npm install -g ${packageName}`);
          console.log(chalk.green('Package installed successfully!'));
        } catch (error) {
          console.error(chalk.red('Failed to install package:'), error);

          const duration = Date.now() - startTime;
          trackCommandError('worker-install', error as Error, duration, {
            packageName,
            stage: 'npm-install',
          });
          await shutdownAnalytics();
          process.exit(1);
        }

        // Find an available port starting from 5555
        let port = options.port;
        if (!port) {
          port = await findAvailablePort(5555);
          console.log(chalk.blue(`Found available port: ${port}`));
        }

        // Get package info to determine the worker name and entry point
        let packageInfo;
        try {
          const {stdout} = await execAsync(`npm view ${packageName} --json`);
          packageInfo = JSON.parse(stdout);
        } catch (error) {
          console.error(chalk.red('Failed to get package info:'), error);

          const duration = Date.now() - startTime;
          trackCommandError('worker-install', error as Error, duration, {
            packageName,
            stage: 'package-info',
          });
          await shutdownAnalytics();
          process.exit(1);
        }

        // Strip scope from package name for display purposes (e.g., @tonk/worker -> worker)
        const commandName =
          options.name || packageName.replace(/^@[^/]+\//, '');

        // Verify the package can be executed directly
        let setupCompleted = false;
        try {
          // Check if the command exists by running a simple help command
          await execAsync(`${commandName} --help`);
          console.log(
            chalk.green(`Verified that '${packageName}' is executable.`),
          );

          // Try running the setup command if it exists
          try {
            console.log(chalk.blue(`Running setup for '${packageName}'...`));
            // Use spawn to allow user interaction during setup
            const {spawn} = await import('node:child_process');

            const setupProcess = spawn(commandName, ['setup'], {
              stdio: 'inherit',
              shell: true,
            });

            // Wait for the setup process to complete
            setupCompleted = await new Promise((resolve, reject) => {
              setupProcess.on('close', code => {
                if (code === 0) {
                  console.log(
                    chalk.green(
                      `Setup completed successfully for '${packageName}'`,
                    ),
                  );
                  resolve(true);
                } else {
                  console.warn(
                    chalk.yellow(`Setup command exited with code ${code}`),
                  );
                  resolve(false);
                }
              });

              setupProcess.on('error', err => {
                reject(err);
              });
            });
          } catch (setupError) {
            console.log(
              chalk.yellow(
                `Setup command not available for '${packageName}', continuing with installation...`,
              ),
            );
          }
        } catch (error) {
          const errorMessage =
            error instanceof Error ? error.message : String(error);
          console.warn(
            chalk.yellow(
              `Warning: Could not verify that '${packageName}' is executable. It may not be properly installed or may not provide a CLI.`,
            ),
          );
          console.warn(chalk.yellow(`Error: ${errorMessage}`));
        }

        // Register the worker with the npm package name
        const worker = await workerManager.register({
          name: packageName, // Use the actual package name for npm resolution
          description:
            packageInfo.description || `Worker from npm package ${packageName}`,
          endpoint: `http://localhost:${port}/tonk`,
          protocol: 'http',
          env: [`WORKER_PORT=${port}`],
          type: 'npm',
        });

        console.log(
          chalk.green(`Worker '${packageName}' registered successfully!`),
        );

        // Start the worker
        console.log(chalk.blue(`Starting worker '${packageName}'...`));
        const success = await workerManager.start(worker.id);

        if (success) {
          console.log(
            chalk.green(`Worker '${packageName}' started successfully!`),
          );
          console.log(chalk.cyan('Worker ID:'), chalk.bold(worker.id));
          console.log(chalk.cyan('Endpoint:'), chalk.bold(worker.endpoint));

          const duration = Date.now() - startTime;
          trackCommandSuccess('worker-install', duration, {
            workerId: worker.id,
            workerName: worker.name,
            packageName,
            port,
            setupCompleted,
            startedSuccessfully: true,
          });
          await shutdownAnalytics();
          process.exit(0);
        } else {
          console.error(chalk.red(`Failed to start worker '${packageName}'.`));

          const duration = Date.now() - startTime;
          trackCommandError(
            'worker-install',
            new Error('Failed to start worker'),
            duration,
            {
              workerId: worker.id,
              packageName,
              stage: 'worker-start',
            },
          );
          await shutdownAnalytics();
          process.exit(1);
        }
      } catch (error) {
        const duration = Date.now() - startTime;
        trackCommandError('worker-install', error as Error, duration, {
          packageName,
          options,
        });
        console.error(chalk.red('Failed to install worker:'), error);
        await shutdownAnalytics();
        process.exit(1);
      }
    });
}

/**
 * Register the init command
 */
export function registerInitCommand(workerCommand: Command): void {
  workerCommand
    .command('init')
    .description('Initialise a new worker configuration file')
    .option(
      '-d, --dir <directory>',
      'Directory to create the configuration file in',
      '.',
    )
    .option('-n, --name <n>', 'Name of the worker')
    .option('-p, --port <port>', 'Port number for the worker', '5555')
    .option('-D, --description <description>', 'Description of the worker')
    .action(async options => {
      const startTime = Date.now();

      try {
        trackCommand('worker-init', {
          hasCustomDir: options.dir !== '.',
          hasName: !!options.name,
          hasCustomPort: options.port !== '5555',
          hasDescription: !!options.description,
        });

        // Prompt for missing options
        const answers = await promptForMissingOptions(options);
        const mergedOptions = {...options, ...answers};

        // Create the directory if it doesn't exist
        const targetDir = path.resolve(mergedOptions.dir);
        let directoryCreated = false;
        if (!fs.existsSync(targetDir)) {
          fs.mkdirSync(targetDir, {recursive: true});
          console.log(chalk.blue(`Created directory: ${targetDir}`));
          directoryCreated = true;
        }

        // Generate the package.json content if it doesn't exist
        const packageJsonPath = path.join(targetDir, 'package.json');
        let packageJsonCreated = false;
        if (!fs.existsSync(packageJsonPath)) {
          const packageJsonContent = generatePackageJsonContent(mergedOptions);
          fs.writeFileSync(packageJsonPath, packageJsonContent);
          console.log(
            chalk.green(`Created package.json at: ${packageJsonPath}`),
          );
          packageJsonCreated = true;
        }

        // Generate the worker.config.js content
        const workerConfigContent = generateWorkerConfigJsContent();
        const workerConfigPath = path.join(targetDir, 'worker.config.js');
        fs.writeFileSync(workerConfigPath, workerConfigContent);

        // Create src directory if it doesn't exist
        const srcDir = path.join(targetDir, 'src');
        let srcDirCreated = false;
        if (!fs.existsSync(srcDir)) {
          fs.mkdirSync(srcDir, {recursive: true});
          console.log(chalk.blue(`Created src directory: ${srcDir}`));
          srcDirCreated = true;
        }

        // Copy CLI template file
        const cliTemplatePath = path.join(
          __dirname,
          'templates',
          'cli.ts.template',
        );
        const cliDestPath = path.join(srcDir, 'cli.ts');
        let cliFileCreated = false;

        if (fs.existsSync(cliTemplatePath)) {
          let cliContent = fs.readFileSync(cliTemplatePath, 'utf-8');

          // Replace template variables
          cliContent = cliContent
            .replace(/{{name}}/g, mergedOptions.name)
            .replace(
              /{{description}}/g,
              mergedOptions.description || 'A Tonk worker',
            )
            .replace(/{{version}}/g, '1.0.0');

          fs.writeFileSync(cliDestPath, cliContent);
          console.log(chalk.green(`Created CLI file at: ${cliDestPath}`));
          cliFileCreated = true;
        } else {
          console.warn(
            chalk.yellow(`CLI template not found at: ${cliTemplatePath}`),
          );
        }

        // Copy index.ts template file
        const indexTemplatePath = path.join(
          __dirname,
          'templates',
          'index.ts.template',
        );
        const indexDestPath = path.join(srcDir, 'index.ts');
        let indexFileCreated = false;

        if (!fs.existsSync(indexDestPath) && fs.existsSync(indexTemplatePath)) {
          let indexContent = fs.readFileSync(indexTemplatePath, 'utf-8');

          // Replace template variables
          indexContent = indexContent
            .replace(/{{name}}/g, mergedOptions.name)
            .replace(
              /{{description}}/g,
              mergedOptions.description || 'A Tonk worker',
            )
            .replace(/{{port}}/g, mergedOptions.port);

          fs.writeFileSync(indexDestPath, indexContent);
          console.log(
            chalk.green(`Created index.ts file at: ${indexDestPath}`),
          );
          indexFileCreated = true;
        } else if (!fs.existsSync(indexTemplatePath)) {
          console.warn(
            chalk.yellow(`Index template not found at: ${indexTemplatePath}`),
          );
        }

        // Copy tsconfig.json template file
        const tsconfigTemplatePath = path.join(
          __dirname,
          'templates',
          'tsconfig.json.template',
        );
        const tsconfigDestPath = path.join(targetDir, 'tsconfig.json');
        let tsconfigFileCreated = false;

        if (
          !fs.existsSync(tsconfigDestPath) &&
          fs.existsSync(tsconfigTemplatePath)
        ) {
          const tsconfigContent = fs.readFileSync(
            tsconfigTemplatePath,
            'utf-8',
          );
          fs.writeFileSync(tsconfigDestPath, tsconfigContent);
          console.log(
            chalk.green(`Created tsconfig.json file at: ${tsconfigDestPath}`),
          );
          tsconfigFileCreated = true;
        } else if (!fs.existsSync(tsconfigTemplatePath)) {
          console.warn(
            chalk.yellow(
              `tsconfig.json template not found at: ${tsconfigTemplatePath}`,
            ),
          );
        }

        console.log(
          chalk.green(
            `Worker configuration file created at: ${workerConfigPath}`,
          ),
        );
        console.log(chalk.blue(`\nYou can now register this worker with:`));
        console.log(chalk.cyan(`  tonk worker register ${targetDir}`));

        const duration = Date.now() - startTime;
        trackCommandSuccess('worker-init', duration, {
          workerName: mergedOptions.name,
          targetDir,
          port: mergedOptions.port,
          directoryCreated,
          packageJsonCreated,
          srcDirCreated,
          cliFileCreated,
          indexFileCreated,
          tsconfigFileCreated,
          filesCreated: [
            packageJsonCreated && 'package.json',
            'worker.config.js',
            srcDirCreated && 'src/',
            cliFileCreated && 'cli.ts',
            indexFileCreated && 'index.ts',
            tsconfigFileCreated && 'tsconfig.json',
          ].filter(Boolean).length,
        });
        await shutdownAnalytics();
        process.exit(0);
      } catch (error) {
        const duration = Date.now() - startTime;
        trackCommandError('worker-init', error as Error, duration, {
          options,
        });
        console.error(
          chalk.red('Failed to initialise worker configuration:'),
          error,
        );
        await shutdownAnalytics();
        process.exit(1);
      }
    });
}

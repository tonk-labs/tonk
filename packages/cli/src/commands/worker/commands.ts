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
    .command('inspect <id>')
    .description('Inspect a specific worker')
    .option('-s, --start', 'Start the worker')
    .option('-S, --stop', 'Stop the worker')
    .option('-c, --config <path>', 'Path to worker configuration file')
    .option('-p, --ping', 'Ping the worker to check its status')
    .action(async (id, options) => {
      try {
        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.get(id);

        if (!worker) {
          console.error(chalk.red(`Worker with ID '${id}' not found.`));
          return;
        }

        // If no options provided, show worker details
        if (
          !options.start &&
          !options.stop &&
          !options.config &&
          !options.ping
        ) {
          displayWorkerDetails(worker);
          return;
        }

        // Handle ping option
        if (options.ping) {
          console.log(
            chalk.blue(
              `Pinging worker '${worker.name}' at ${worker.endpoint}...`,
            ),
          );

          // Check worker health
          const isHealthy = await workerManager.checkHealth(id);

          if (isHealthy) {
            console.log(chalk.green(`Worker '${worker.name}' is active!`));
          } else {
            console.log(
              chalk.red(`Worker '${worker.name}' is not responding.`),
            );
          }
          return;
        }

        // Handle start option
        if (options.start) {
          console.log(chalk.blue(`Starting worker '${worker.name}'...`));
          await workerManager.start(id);
          console.log(
            chalk.green(`Worker '${worker.name}' started successfully!`),
          );
          return;
        }

        // Handle stop option
        if (options.stop) {
          console.log(chalk.blue(`Stopping worker '${worker.name}'...`));
          await workerManager.stop(id);
          console.log(
            chalk.green(`Worker '${worker.name}' stopped successfully!`),
          );
          return;
        }

        // Handle config option
        if (options.config) {
          await updateWorkerConfig(workerManager, worker.id, options.config);
        }
      } catch (error) {
        console.error(chalk.red('Failed to manage worker:'), error);
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
      try {
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
          return;
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
            `\nUse '${chalk.bold('tonk worker inspect <id>')}' to view details of a specific worker.`,
          ),
        );
      } catch (error) {
        console.error(chalk.red('Failed to list workers:'), error);
      }
    });
}

/**
 * Register the remove command
 */
export function registerRemoveCommand(workerCommand: Command): void {
  workerCommand
    .command('rm <id>')
    .description('Remove a registered worker')
    .action(async id => {
      try {
        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.get(id);

        if (!worker) {
          console.error(chalk.red(`Worker with ID '${id}' not found.`));
          return;
        }

        // Remove worker
        await workerManager.remove(id);
        console.log(
          chalk.green(`Worker '${worker.name}' (${id}) removed successfully.`),
        );
      } catch (error) {
        console.error(chalk.red('Failed to remove worker:'), error);
      }
    });
}

/**
 * Register the ping command
 */
export function registerPingCommand(workerCommand: Command): void {
  workerCommand
    .command('ping <id>')
    .description('Ping a worker to check its status')
    .action(async id => {
      try {
        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.get(id);

        if (!worker) {
          console.error(chalk.red(`Worker with ID '${id}' not found.`));
          return;
        }

        console.log(
          chalk.blue(
            `Pinging worker '${worker.name}' at ${worker.endpoint}...`,
          ),
        );

        // Check worker health
        const isHealthy = await workerManager.checkHealth(id);

        if (isHealthy) {
          console.log(chalk.green(`Worker '${worker.name}' is active!`));
        } else {
          console.log(chalk.red(`Worker '${worker.name}' is not responding.`));
        }
      } catch (error) {
        console.error(chalk.red('Failed to ping worker:'), error);
      }
    });
}

/**
 * Register the start command
 */
export function registerStartCommand(workerCommand: Command): void {
  workerCommand
    .command('start <id>')
    .description('Start a worker')
    .action(async id => {
      try {
        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.get(id);

        if (!worker) {
          console.error(chalk.red(`Worker with ID '${id}' not found.`));
          return;
        }

        console.log(chalk.blue(`Starting worker '${worker.name}'...`));
        await workerManager.start(id);
        console.log(
          chalk.green(`Worker '${worker.name}' started successfully!`),
        );
      } catch (error) {
        console.error(chalk.red('Failed to start worker:'), error);
      }
    });
}

/**
 * Register the stop command
 */
export function registerStopCommand(workerCommand: Command): void {
  workerCommand
    .command('stop <id>')
    .description('Stop a worker')
    .action(async id => {
      try {
        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.get(id);

        if (!worker) {
          console.error(chalk.red(`Worker with ID '${id}' not found.`));
          return;
        }

        console.log(chalk.blue(`Stopping worker '${worker.name}'...`));
        await workerManager.stop(id);
        console.log(
          chalk.green(`Worker '${worker.name}' stopped successfully!`),
        );
      } catch (error) {
        console.error(chalk.red('Failed to stop worker:'), error);
      }
    });
}

/**
 * Register the logs command
 */
export function registerLogsCommand(workerCommand: Command): void {
  workerCommand
    .command('logs <id>')
    .description('View logs for a worker')
    .option('-f, --follow', 'Follow log output')
    .option('-l, --lines <n>', 'Number of lines to show', '100')
    .option('-e, --error', 'Show only error logs')
    .option('-o, --out', 'Show only standard output logs')
    .action(async (id, options) => {
      try {
        // Create worker manager
        const workerManager = new TonkWorkerManager();

        // Get worker
        const worker = await workerManager.get(id);

        if (!worker) {
          console.error(chalk.red(`Worker with ID '${id}' not found.`));
          return;
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
            process.on('SIGINT', () => {
              child.kill();
              process.exit(0);
            });

            // Wait for the child process to exit
            await new Promise(resolve => {
              child.on('exit', resolve);
            });
          }
        } catch (error) {
          console.error(chalk.red('Failed to fetch logs:'), error);
        }
      } catch (error) {
        console.error(chalk.red('Failed to fetch worker logs:'), error);
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
      try {
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
          return;
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
          return;
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
          return;
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
          return;
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
      } catch (error) {
        console.error(chalk.red('Failed to register worker:'), error);
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
      try {
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
          return;
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
          return;
        }

        // Register the worker with the npm package name (use display name in description)
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
          chalk.green(`Worker '${worker.name}' registered successfully!`),
        );

        // Start the worker
        console.log(chalk.blue(`Starting worker '${worker.name}'...`));
        const success = await workerManager.start(worker.id);

        if (success) {
          console.log(
            chalk.green(`Worker '${worker.name}' started successfully!`),
          );
          console.log(chalk.cyan('Worker ID:'), chalk.bold(worker.id));
          console.log(chalk.cyan('Endpoint:'), chalk.bold(worker.endpoint));
        } else {
          console.error(chalk.red(`Failed to start worker '${worker.name}'.`));
        }
      } catch (error) {
        console.error(chalk.red('Failed to install worker:'), error);
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
      try {
        // Prompt for missing options
        const answers = await promptForMissingOptions(options);
        const mergedOptions = {...options, ...answers};

        // Create the directory if it doesn't exist
        const targetDir = path.resolve(mergedOptions.dir);
        if (!fs.existsSync(targetDir)) {
          fs.mkdirSync(targetDir, {recursive: true});
          console.log(chalk.blue(`Created directory: ${targetDir}`));
        }

        // Generate the package.json content if it doesn't exist
        const packageJsonPath = path.join(targetDir, 'package.json');
        if (!fs.existsSync(packageJsonPath)) {
          const packageJsonContent = generatePackageJsonContent(mergedOptions);
          fs.writeFileSync(packageJsonPath, packageJsonContent);
          console.log(
            chalk.green(`Created package.json at: ${packageJsonPath}`),
          );
        }

        // Generate the worker.config.js content
        const workerConfigContent = generateWorkerConfigJsContent();
        const workerConfigPath = path.join(targetDir, 'worker.config.js');
        fs.writeFileSync(workerConfigPath, workerConfigContent);

        // Create src directory if it doesn't exist
        const srcDir = path.join(targetDir, 'src');
        if (!fs.existsSync(srcDir)) {
          fs.mkdirSync(srcDir, {recursive: true});
          console.log(chalk.blue(`Created src directory: ${srcDir}`));
        }

        // Copy CLI template file
        const cliTemplatePath = path.join(
          __dirname,
          'templates',
          'cli.ts.template',
        );
        const cliDestPath = path.join(srcDir, 'cli.ts');

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
      } catch (error) {
        console.error(
          chalk.red('Failed to initialise worker configuration:'),
          error,
        );
      }
    });
}

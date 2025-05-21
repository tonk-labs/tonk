import {Command} from 'commander';
import chalk from 'chalk';
import fs from 'node:fs';
import path from 'node:path';
import * as YAML from 'yaml';
import Table from 'cli-table3';
import inquirer from 'inquirer';
import {TonkWorkerManager} from '../lib/workerManager.js';

export const workerCommand = new Command('worker')
  .description('Manage Tonk workers')
  .action(async () => {
    // Display help when no subcommand is provided
    workerCommand.help();
  });

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
      if (!options.start && !options.stop && !options.config && !options.ping) {
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
          console.log(chalk.red(`Worker '${worker.name}' is not responding.`));
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
          chalk.cyan('Type'),
          chalk.cyan('Endpoint'),
          chalk.cyan('Protocol'),
          chalk.cyan('Status'),
          chalk.cyan('Last Seen'),
        ],
        colWidths: [24, 20, 10, 30, 10, 10, 20],
      });

      // Add workers to table
      workers.forEach(worker => {
        const lastSeen = worker.status.lastSeen
          ? new Date(worker.status.lastSeen).toLocaleString()
          : 'Never';

        table.push([
          worker.id,
          worker.name,
          worker.config.type || 'custom',
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
        chalk.blue(`Pinging worker '${worker.name}' at ${worker.endpoint}...`),
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
      console.log(chalk.green(`Worker '${worker.name}' started successfully!`));
    } catch (error) {
      console.error(chalk.red('Failed to start worker:'), error);
    }
  });

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
      console.log(chalk.green(`Worker '${worker.name}' stopped successfully!`));
    } catch (error) {
      console.error(chalk.red('Failed to stop worker:'), error);
    }
  });

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

workerCommand
  .command('register')
  .description('Register a worker with Tonk')
  .argument('[dir]', 'Path to worker directory (defaults to current directory)')
  .action(async (dir = '.') => {
    try {
      // Find the worker.yaml file
      const workerDir = await findWorkerRoot(dir);

      if (!workerDir) {
        console.error(
          chalk.red(
            `No worker.yaml file found in ${path.resolve(dir)} or its parent directories.`,
          ),
        );
        console.log(
          chalk.yellow(
            'Make sure you are in a valid worker directory or specify the path to one.',
          ),
        );
        return;
      }

      console.log(chalk.blue(`Found worker directory at: ${workerDir}`));

      // Read the worker.yaml file
      const workerYamlPath = path.join(workerDir, 'worker.yaml');
      const workerConfig = await readWorkerConfig(workerYamlPath);

      if (!workerConfig) {
        console.error(
          chalk.red(`Failed to parse worker.yaml file at ${workerYamlPath}`),
        );
        return;
      }

      // Create worker manager
      const workerManager = new TonkWorkerManager();

      // Register worker using the YAML config
      const worker = await workerManager.registerFromYamlConfig(workerConfig);

      console.log(
        chalk.green(`Worker '${worker.name}' registered successfully!`),
      );
      console.log(chalk.cyan('Worker ID:'), chalk.bold(worker.id));
      console.log(chalk.cyan('Endpoint:'), chalk.bold(worker.endpoint));
      console.log(chalk.cyan('Protocol:'), chalk.bold(worker.protocol));
      console.log(
        chalk.cyan('Status:'),
        worker.status.active ? chalk.green('Active') : chalk.yellow('Inactive'),
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

      // Generate the YAML content
      const yamlContent = generateYamlContent(mergedOptions);

      // Write the YAML file
      const yamlPath = path.join(targetDir, 'worker.yaml');
      fs.writeFileSync(yamlPath, yamlContent);

      console.log(
        chalk.green(`Worker configuration file created at: ${yamlPath}`),
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

// Helper function to display worker details
function displayWorkerDetails(worker: any) {
  console.log(chalk.bold(`Worker Details: ${worker.name}`));
  console.log(chalk.cyan('ID:'), worker.id);
  console.log(chalk.cyan('Name:'), worker.name);
  console.log(
    chalk.cyan('Description:'),
    worker.description || '(No description)',
  );
  console.log(chalk.cyan('Type:'), worker.config.type || 'custom');
  console.log(chalk.cyan('Endpoint:'), worker.endpoint);
  console.log(chalk.cyan('Protocol:'), worker.protocol);
  console.log(
    chalk.cyan('Status:'),
    worker.status.active ? chalk.green('Active') : chalk.yellow('Inactive'),
  );

  if (worker.status.lastSeen) {
    console.log(
      chalk.cyan('Last Seen:'),
      new Date(worker.status.lastSeen).toLocaleString(),
    );
  } else {
    console.log(chalk.cyan('Last Seen:'), 'Never');
  }

  console.log(
    chalk.cyan('Created:'),
    new Date(worker.createdAt).toLocaleString(),
  );
  console.log(
    chalk.cyan('Updated:'),
    new Date(worker.updatedAt).toLocaleString(),
  );

  if (Object.keys(worker.env).length > 0) {
    console.log(chalk.cyan('\nEnvironment Variables:'));
    Object.entries(worker.env).forEach(([key, value]) => {
      console.log(`  ${key}=${value}`);
    });
  }

  if (Object.keys(worker.config).length > 0) {
    console.log(chalk.cyan('\nConfiguration:'));
    console.log(JSON.stringify(worker.config, null, 2));
  }
}

// Helper function to update worker configuration
async function updateWorkerConfig(
  workerManager: TonkWorkerManager,
  id: string,
  configPath: string,
) {
  try {
    // Check if config file exists
    if (!fs.existsSync(configPath)) {
      console.error(chalk.red(`Configuration file not found: ${configPath}`));
      return;
    }

    // Read config file
    const configContent = fs.readFileSync(configPath, 'utf-8');
    let config: Record<string, any>;

    try {
      config = JSON.parse(configContent);
    } catch (error) {
      console.error(chalk.red('Invalid JSON in configuration file:'), error);
      return;
    }

    // Get worker
    const worker = await workerManager.get(id);

    if (!worker) {
      console.error(chalk.red(`Worker with ID '${id}' not found.`));
      return;
    }

    // Update worker config
    await workerManager.update(id, {
      config: {...worker.config, ...config},
    });

    console.log(
      chalk.green(
        `Worker '${worker.name}' configuration updated successfully!`,
      ),
    );
  } catch (error) {
    console.error(chalk.red('Failed to update worker configuration:'), error);
  }
}

/**
 * Find the worker root directory by looking for worker.yaml
 * Searches in the given directory and parent directories
 */
async function findWorkerRoot(startDir: string): Promise<string | null> {
  try {
    let currentDir = path.resolve(startDir);
    const rootDir = path.parse(currentDir).root;

    // Search up to the root directory
    while (currentDir !== rootDir) {
      const workerYamlPath = path.join(currentDir, 'worker.yaml');

      if (fs.existsSync(workerYamlPath)) {
        return currentDir;
      }

      // Move up one directory
      const parentDir = path.dirname(currentDir);

      // If we've reached the root, stop searching
      if (parentDir === currentDir) {
        break;
      }

      currentDir = parentDir;
    }

    return null;
  } catch (error) {
    console.error('Error finding worker root:', error);
    return null;
  }
}

/**
 * Read and parse the worker.yaml file
 */
async function readWorkerConfig(configPath: string): Promise<any | null> {
  try {
    if (!fs.existsSync(configPath)) {
      return null;
    }

    const fileContent = fs.readFileSync(configPath, 'utf-8');
    return YAML.parse(fileContent);
  } catch (error) {
    console.error('Error reading worker config:', error);
    return null;
  }
}

// Helper function to prompt for missing options (for init command)
async function promptForMissingOptions(options: any) {
  const questions = [];

  if (!options.name) {
    questions.push({
      type: 'input',
      name: 'name',
      message: 'Enter worker name:',
      validate: (input: string) =>
        input.trim() !== '' ? true : 'Name is required',
    });
  }

  if (!options.port) {
    questions.push({
      type: 'input',
      name: 'port',
      message: 'Enter port number for the worker:',
      default: '5555',
      validate: (input: string) =>
        /^\d+$/.test(input) ? true : 'Port must be a number',
    });
  }

  return inquirer.prompt(questions);
}

// Helper function to generate YAML content (for init command)
function generateYamlContent(options: any): string {
  const port = options.port || '5555';

  return `# Worker Configuration

# Worker Information
name: ${options.name}
description: Tonk worker
version: 1.0.0

# Connection Details
endpoint: http://localhost:${port}/tonk
protocol: http

# Health Check Configuration
healthCheck:
  endpoint: http://localhost:${port}/health
  method: GET
  interval: 30000
  timeout: 5000

# Process Management
process:
  script: ./dist/index.js
  cwd: ./
  instances: 1
  autorestart: true
  watch: false
  max_memory_restart: 500M
  env:
    NODE_ENV: production

# Additional Configuration
config: {}
`;
}

import fs from 'node:fs';
import chalk from 'chalk';
import inquirer from 'inquirer';
import {TonkWorkerManager} from '../../../lib/workerManager.js';

/**
 * Read and parse the package.json file
 */
export async function readPackageJson(
  packageJsonPath: string,
): Promise<any | null> {
  try {
    if (!fs.existsSync(packageJsonPath)) {
      return null;
    }

    const fileContent = fs.readFileSync(packageJsonPath, 'utf-8');
    return JSON.parse(fileContent);
  } catch (error) {
    console.error('Error reading package.json:', error);
    return null;
  }
}

/**
 * Read and parse the worker.config.js file
 */
export async function readWorkerConfigJs(
  configPath: string,
): Promise<any | null> {
  try {
    if (!fs.existsSync(configPath)) {
      return null;
    }

    // For JavaScript files, we can use dynamic import
    const configModule = await import(`file://${configPath}`);
    return configModule.default || configModule;
  } catch (error) {
    console.error('Error reading worker.config.js:', error);
    return null;
  }
}

/**
 * Generate package.json content for a worker
 */
export function generatePackageJsonContent(options: any): string {
  const name = options.name || 'tonk-worker';
  const description = options.description || 'A Tonk worker';

  return JSON.stringify(
    {
      name,
      version: '1.0.0',
      description,
      main: 'dist/index.js',
      bin: {
        [name]: 'dist/cli.js',
      },
      scripts: {
        start: 'node dist/index.js',
        build: 'tsc',
        dev: 'ts-node src/index.ts',
      },
      dependencies: {
        '@tonk/worker': '^0.1.0',
      },
      devDependencies: {
        typescript: '^5.0.0',
        'ts-node': '^10.9.1',
        '@types/node': '^18.0.0',
      },
    },
    null,
    2,
  );
}

/**
 * Generate worker.config.js content
 */
export function generateWorkerConfigJsContent(): string {
  return `/**
 * Tonk Worker Configuration
 */
module.exports = {
  // Runtime configuration
  runtime: {
    port: {{port}},
    healthCheck: {
      endpoint: '/health',
      method: 'GET',
      interval: 30000,
      timeout: 5000,
    },
  },

  // Process management
  process: {
    script: './dist/cli.js',
    cwd: './',
    instances: 1,
    autorestart: true,
    watch: false,
    max_memory_restart: '500M',
    env: {
      NODE_ENV: 'production',
    },
  },

  // CLI configuration
  cli: {
    script: './dist/cli.js',
    command: 'start',
    args: ['--port', '{{port}}'],
  },

  // Data schema
  // This section defines the schemas for data stored in keepsync
  // Can be used to validate data before storing it
  schemas: {
    // Define schemas for different document types
    documents: {
      // Main document schema
      default: {},

      // Additional document types can be defined here
      // For example:
      //
      // specialDocument: {
      //   type: "object",
      //   properties: { ... }
      // }
    },
  },

  // Additional configuration
  config: {},
};
`;
}

/**
 * Helper function to update worker configuration
 */
export async function updateWorkerConfig(
  workerManager: TonkWorkerManager,
  id: string,
  configPath: string,
): Promise<void> {
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
 * Helper function to prompt for missing options (for init command)
 */
export async function promptForMissingOptions(options: any) {
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

  if (!options.description) {
    questions.push({
      type: 'input',
      name: 'description',
      message: 'Enter worker description:',
      default: 'A Tonk worker',
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

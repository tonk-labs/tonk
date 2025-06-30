import {Command} from 'commander';
import chalk from 'chalk';
import inquirer from 'inquirer';
import ora from 'ora';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
  shutdownAnalytics,
} from '../utils/analytics.js';
// Lazy-load authHook to prevent initialization on import
async function getAuthHook() {
  const {authHook} = await import('../lib/tonkAuth.js');
  return authHook;
}
// Lazy-load tonkAuth to get auth token
async function getTonkAuth() {
  const {tonkAuth} = await import('../lib/tonkAuth.js');
  return tonkAuth;
}
import {getDeploymentServiceUrl, config} from '../config/environment.js';

interface ServerOptions {
  name?: string;
  region?: string;
  memory?: string;
  cpus?: string;
  remote?: boolean;
}

/**
 * Checks if the deployment service is reachable
 */
async function checkDeploymentService(): Promise<void> {
  try {
    const response = await fetch(`${getDeploymentServiceUrl()}/health`);
    if (!response.ok) {
      throw new Error(`Service returned ${response.status}`);
    }
  } catch (error) {
    console.error(
      chalk.red('Error: Tonk deployment service is not available.'),
    );
    console.log(
      chalk.yellow('Please reach out for support or try again later.'),
    );
    await shutdownAnalytics();
    process.exitCode = 1;
    return;
  }
}

/**
 * Gets server name from user if not provided
 */
async function getServerName(serverName?: string): Promise<string> {
  if (serverName) {
    return serverName;
  }

  const {inputName} = await inquirer.prompt([
    {
      type: 'input',
      name: 'inputName',
      message: 'Enter your Tonk server name:',
      validate: (input: string) => {
        if (!input || input.trim().length === 0) {
          return 'Server name is required';
        }
        return true;
      },
    },
  ]);

  return inputName.trim();
}

/**
 * Creates a new Tonk server
 */
async function createServer(
  serverName: string,
  options: ServerOptions,
): Promise<void> {
  const spinner = ora('Creating Tonk server...').start();

  try {

    // Get auth token
    const tonkAuth = await getTonkAuth();
    const authToken = await tonkAuth.getAuthToken();
    if (!authToken) {
      throw new Error('Failed to get authentication token');
    }

    // Prepare form data for server creation
    const formData = new FormData();
    formData.append(
      'serverData',
      JSON.stringify({
        serverName,
        region: options.region || 'ord',
        memory: options.memory || '1gb',
        cpus: options.cpus || '1',
        remote: options.remote || false,
        deployType: 'server',
      }),
    );

    // Send server creation request
    spinner.text = 'Creating server...';
    const response = await fetch(`${getDeploymentServiceUrl()}/create-server`, {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${authToken}`,
      },
      body: formData,
    });

    const result: any = await response.json();

    if (!response.ok || !result.success) {
      throw new Error(result.error || 'Server creation failed');
    }

    spinner.succeed('Server created successfully!');

    console.log(chalk.green(`🚀 Your Tonk server is ready!`));
    console.log(chalk.blue(`   URL: ${result.serverUrl}`));
    console.log(
      chalk.yellow(
        `   Use this server name for future bundle deployments: ${serverName}`,
      ),
    );
  } catch (error) {
    spinner.fail('Server creation failed');
    throw error;
  }
}

/**
 * Lists bundles on a remote server
 */
async function listRemoteBundles(
  serverName: string,
): Promise<void> {
  // Get auth token
  const tonkAuth = await getTonkAuth();
  const authToken = await tonkAuth.getAuthToken();
  if (!authToken) {
    throw new Error('Failed to get authentication token');
  }

  const formData = new FormData();
  formData.append(
    'requestData',
    JSON.stringify({
      serverName,
      action: 'ls',
    }),
  );

  const response = await fetch(`${getDeploymentServiceUrl()}/server-action`, {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${authToken}`,
    },
    body: formData,
  });

  const result: any = await response.json();

  if (!response.ok || !result.success) {
    throw new Error(result.error || 'Failed to list bundles');
  }

  if (result.bundles && result.bundles.length > 0) {
    console.log(chalk.green(`📦 Bundles on server "${serverName}":`));
    result.bundles.forEach((bundle: string) => {
      console.log(chalk.blue(`  • ${bundle}`));
    });
  } else {
    console.log(chalk.yellow(`No bundles found on server "${serverName}"`));
  }
}

/**
 * Lists running bundles on a remote server
 */
async function listRunningBundles(
  serverName: string,
): Promise<void> {
  // Get auth token
  const tonkAuth = await getTonkAuth();
  const authToken = await tonkAuth.getAuthToken();
  if (!authToken) {
    throw new Error('Failed to get authentication token');
  }

  const formData = new FormData();
  formData.append(
    'requestData',
    JSON.stringify({
      serverName,
      action: 'ps',
    }),
  );

  const response = await fetch(`${getDeploymentServiceUrl()}/server-action`, {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${authToken}`,
    },
    body: formData,
  });

  const result: any = await response.json();

  if (!response.ok || !result.success) {
    throw new Error(result.error || 'Failed to list running bundles');
  }

  if (result.processes && result.processes.length > 0) {
    console.log(chalk.green(`🏃 Running bundles on server "${serverName}":`));
    result.processes.forEach((process: any) => {
      console.log(chalk.blue(`  • ${process.bundleName} (${process.id})`));
      console.log(chalk.gray(`    Route: ${process.route}`));
      console.log(chalk.gray(`    Status: ${process.status}`));
      if (process.url) {
        console.log(chalk.gray(`    URL: ${process.url}`));
      }
      console.log('');
    });
  } else {
    console.log(chalk.yellow(`No running bundles on server "${serverName}"`));
  }
}

/**
 * Deletes a bundle from a remote server
 */
async function deleteRemoteBundle(
  bundleName: string,
  serverName: string,
): Promise<void> {
  // Get auth token
  const tonkAuth = await getTonkAuth();
  const authToken = await tonkAuth.getAuthToken();
  if (!authToken) {
    throw new Error('Failed to get authentication token');
  }

  const formData = new FormData();
  formData.append(
    'deleteData',
    JSON.stringify({
      bundleName,
      serverName,
      deployType: 'delete',
    }),
  );

  const response = await fetch(`${getDeploymentServiceUrl()}/delete-bundle`, {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${authToken}`,
    },
    body: formData,
  });

  const result: any = await response.json();

  if (!response.ok || !result.success) {
    throw new Error(result.error || 'Failed to delete bundle');
  }

  console.log(chalk.green('Bundle deleted successfully!'));
  console.log(chalk.green(result.message));
}

/**
 * Server create command handler
 */
async function handleServerCreateCommand(
  options: ServerOptions,
): Promise<void> {
  const startTime = Date.now();

  try {
    // Run auth check (replaces the preAction hook)
    const authHook = await getAuthHook();
    await authHook();

    trackCommand('server-create', {
      name: options.name,
      region: options.region,
      memory: options.memory,
      cpus: options.cpus,
      remote: options.remote,
    });

    console.log(chalk.cyan('🔧 Tonk Server Create\n'));

    // Check prerequisites
    console.log(chalk.blue('Checking prerequisites...'));
    await checkDeploymentService();

    // Determine server name
    let serverName = options.name;
    if (!serverName) {
      const {inputName} = await inquirer.prompt([
        {
          type: 'input',
          name: 'inputName',
          message: 'Enter server name:',
          validate: (input: string) => {
            if (!input || input.trim().length === 0) {
              return 'Server name is required';
            }
            return true;
          },
        },
      ]);
      serverName = inputName
        .trim()
        .replace(/[^a-zA-Z0-9-]/g, '-')
        .toLowerCase();
    }

    console.log(chalk.green(`✓ Server name: ${serverName}`));

    // Confirm server creation
    const {confirm} = await inquirer.prompt([
      {
        type: 'confirm',
        name: 'confirm',
        message: `Create Tonk server "${serverName}"?`,
        default: true,
      },
    ]);

    if (!confirm) {
      console.log(chalk.yellow('Server creation cancelled'));
      await shutdownAnalytics();
      return;
    }

    // Create the server
    await createServer(serverName!, options);

    const duration = Date.now() - startTime;
    trackCommandSuccess('server-create', duration, {
      serverName,
      region: options.region,
      memory: options.memory,
      cpus: options.cpus,
      remote: options.remote,
    });
    await shutdownAnalytics();
  } catch (error) {
    const duration = Date.now() - startTime;
    trackCommandError('server-create', error as Error, duration, {
      name: options.name,
      region: options.region,
    });

    console.error(
      chalk.red(
        `Error: ${error instanceof Error ? error.message : String(error)}`,
      ),
    );
    await shutdownAnalytics();
    process.exitCode = 1;
  }
}

// Server create command
const serverCreateCommand = new Command('create')
  .description('Create a new Tonk server')
  .option('-n, --name <name>', 'Name for the server')
  .option(
    '-r, --region <region>',
    'Region to deploy to',
    config.deployment.defaultRegion,
  )
  .option(
    '-m, --memory <memory>',
    'Memory allocation (e.g., 256mb, 1gb)',
    config.deployment.defaultMemory,
  )
  .option('-c, --cpus <cpus>', 'Number of CPUs', config.deployment.defaultCpus)
  .option(
    '--remote',
    'Use remote Docker build (slower but works with limited local resources)',
  )
  .action(handleServerCreateCommand);

// Server ls command
const serverLsCommand = new Command('ls')
  .description('List bundles on a remote Tonk server')
  .option('-s, --server <server>', 'Name of the Tonk server')
  .action(async options => {
    const startTime = Date.now();

    try {
      // Run auth check (replaces the preAction hook)
      const authHook = await getAuthHook();
      await authHook();

      trackCommand('server-ls', {
        server: options.server,
      });

      console.log(chalk.cyan('📋 Server Bundle List\n'));

      // Check prerequisites
      await checkDeploymentService();

      // Get server name
      const serverName = await getServerName(options.server);

      // List bundles
      await listRemoteBundles(serverName);

      const duration = Date.now() - startTime;
      trackCommandSuccess('server-ls', duration, {
        serverName,
      });
      await shutdownAnalytics();
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('server-ls', error as Error, duration, {
        server: options.server,
      });

      console.error(
        chalk.red(
          `Error: ${error instanceof Error ? error.message : String(error)}`,
        ),
      );
      process.exitCode = 1;
    }
  });

// Server ps command
const serverPsCommand = new Command('ps')
  .description('List running bundles on a remote Tonk server')
  .option('-s, --server <server>', 'Name of the Tonk server')
  .action(async options => {
    const startTime = Date.now();

    try {
      // Run auth check (replaces the preAction hook)
      const authHook = await getAuthHook();
      await authHook();

      trackCommand('server-ps', {
        server: options.server,
      });

      console.log(chalk.cyan('🏃 Server Running Bundles\n'));

      // Check prerequisites
      await checkDeploymentService();

      // Get server name
      const serverName = await getServerName(options.server);

      // List running bundles
      await listRunningBundles(serverName);

      const duration = Date.now() - startTime;
      trackCommandSuccess('server-ps', duration, {
        serverName,
      });
      await shutdownAnalytics();
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('server-ps', error as Error, duration, {
        server: options.server,
      });

      console.error(
        chalk.red(
          `Error: ${error instanceof Error ? error.message : String(error)}`,
        ),
      );
      process.exitCode = 1;
    }
  });

// Server rm command
const serverRmCommand = new Command('rm')
  .alias('delete')
  .description('Delete a bundle from a remote Tonk server')
  .option('-s, --server <server>', 'Name of the Tonk server')
  .argument('<bundleName>', 'Name of the bundle to delete')
  .action(async (bundleName, options) => {
    const startTime = Date.now();

    try {
      // Run auth check (replaces the preAction hook)
      const authHook = await getAuthHook();
      await authHook();

      trackCommand('server-rm', {
        bundleName,
        server: options.server,
      });

      console.log(chalk.cyan('🗑️  Server Bundle Delete\n'));

      // Check prerequisites
      await checkDeploymentService();

      // Get server name
      const serverName = await getServerName(options.server);

      // Confirm deletion
      const {confirm} = await inquirer.prompt([
        {
          type: 'confirm',
          name: 'confirm',
          message: `Delete bundle "${bundleName}" from server "${serverName}"?`,
          default: false,
        },
      ]);

      if (!confirm) {
        console.log(chalk.yellow('Bundle deletion cancelled'));
        await shutdownAnalytics();
        return;
      }

      // Delete bundle
      await deleteRemoteBundle(bundleName, serverName);

      const duration = Date.now() - startTime;
      trackCommandSuccess('server-rm', duration, {
        bundleName,
        serverName,
      });
      await shutdownAnalytics();
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('server-rm', error as Error, duration, {
        bundleName,
        server: options.server,
      });

      console.error(
        chalk.red(
          `Error: ${error instanceof Error ? error.message : String(error)}`,
        ),
      );
      process.exitCode = 1;
    }
  });

// Main server command
export const serverCommand = new Command('server').description(
  'Manage Tonk servers',
);

serverCommand.addCommand(serverCreateCommand);
serverCommand.addCommand(serverLsCommand);
serverCommand.addCommand(serverPsCommand);
serverCommand.addCommand(serverRmCommand);

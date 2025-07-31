import { Command } from 'commander';
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
  const { authHook } = await import('../lib/tonkAuth.js');
  return authHook;
}
// Lazy-load tonkAuth to get auth token
async function getTonkAuth() {
  const { tonkAuth } = await import('../lib/tonkAuth.js');
  return tonkAuth;
}
import { getDeploymentServiceUrl, config } from '../config/environment.js';
import { fetchUserServers } from '../utils/serverUtils.js';

// Privileged subdomain names that cannot be used as server names
const PRIVILEGED_SUBDOMAINS = [
  'accounts',
  'clerk',
  'clkmail',
  'dashboard',
  'tinyfoot',
];

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
  } catch {
    console.error(
      chalk.red('Error: Tonk deployment service is not available.')
    );
    console.log(
      chalk.yellow('Please reach out for support or try again later.')
    );
    await shutdownAnalytics();
    process.exitCode = 1;
    return;
  }
}

/**
 * Validates that a server name is not a privileged subdomain
 */
function validateServerName(serverName: string): string | boolean {
  if (!serverName || serverName.trim().length === 0) {
    return 'Server name is required';
  }

  if (PRIVILEGED_SUBDOMAINS.includes(serverName.toLowerCase())) {
    return `Server name "${serverName}" is reserved and cannot be used. Please choose a different name.`;
  }

  return true;
}

/**
 * Gets server name from user if not provided
 */
async function getServerName(serverName?: string): Promise<string> {
  if (serverName) {
    return serverName;
  }

  const { inputName } = await inquirer.prompt([
    {
      type: 'input',
      name: 'inputName',
      message: 'Enter your Tonk server name:',
      validate: validateServerName,
    },
  ]);

  return inputName.trim();
}

/**
 * Deletes a server
 */
async function deleteServer(serverName: string): Promise<void> {
  // Get auth token
  const tonkAuth = await getTonkAuth();
  const authToken = await tonkAuth.getAuthToken();
  if (!authToken) {
    throw new Error('Failed to get authentication token');
  }

  const formData = new FormData();
  formData.append(
    'serverData',
    JSON.stringify({
      serverName,
    })
  );

  const response = await fetch(`${getDeploymentServiceUrl()}/delete-server`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${authToken}`,
    },
    body: formData,
  });

  const result: any = await response.json();

  if (!response.ok || !result.success) {
    throw new Error(result.error || 'Failed to delete server');
  }

  console.log(chalk.green('Server deleted successfully!'));
  console.log(chalk.green(result.message));
}

/**
 * Creates a new Tonk server
 */
async function createServer(
  serverName: string,
  options: ServerOptions
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
      })
    );

    // Send server creation request
    spinner.text = 'Creating server...';
    const response = await fetch(`${getDeploymentServiceUrl()}/create-server`, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${authToken}`,
      },
      body: formData,
    });

    const result: any = await response.json();

    if (!response.ok || !result.success) {
      const errorMessage = result.error || 'Server creation failed';

      // Check for name conflict errors
      if (
        errorMessage.toLowerCase().includes('already exists') ||
        errorMessage.toLowerCase().includes('name is taken') ||
        errorMessage.toLowerCase().includes('name already taken') ||
        errorMessage.toLowerCase().includes('already in use') ||
        errorMessage.toLowerCase().includes('conflict')
      ) {
        throw new Error(
          `Server name "${serverName}" is already taken. Please choose a different server name and try again.`
        );
      }

      throw new Error(errorMessage);
    }

    spinner.succeed('Server created successfully!');

    console.log(chalk.green(`🚀 Your Tonk server is ready!`));
    console.log(chalk.blue(`   URL: ${result.serverUrl}`));
    console.log(
      chalk.yellow(
        `   Use this server name for future bundle deployments: ${serverName}`
      )
    );
  } catch (error) {
    spinner.fail('Server creation failed');
    throw error;
  }
}

/**
 * Lists bundles on a remote server
 */
async function listRemoteBundles(serverName: string): Promise<void> {
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
    })
  );

  const response = await fetch(`${getDeploymentServiceUrl()}/server-action`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${authToken}`,
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
async function listRunningBundles(serverName: string): Promise<void> {
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
    })
  );

  const response = await fetch(`${getDeploymentServiceUrl()}/server-action`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${authToken}`,
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
  serverName: string
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
    })
  );

  const response = await fetch(`${getDeploymentServiceUrl()}/delete-bundle`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${authToken}`,
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
  options: ServerOptions
): Promise<void> {
  const startTime = Date.now();
  let tonkAuth: any = null;

  try {
    // Run auth check (replaces the preAction hook)
    const authHook = await getAuthHook();
    await authHook();

    // Get TonkAuth instance for cleanup
    tonkAuth = await getTonkAuth();

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
      const { inputName } = await inquirer.prompt([
        {
          type: 'input',
          name: 'inputName',
          message: 'Enter server name:',
          validate: validateServerName,
        },
      ]);
      serverName = inputName
        .trim()
        .replace(/[^a-zA-Z0-9-]/g, '-')
        .toLowerCase();
    } else {
      // Validate provided server name
      const validation = validateServerName(serverName);
      if (validation !== true) {
        throw new Error(validation as string);
      }
    }

    console.log(chalk.green(`✓ Server name: ${serverName}`));

    // Confirm server creation
    const { confirm } = await inquirer.prompt([
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
    if (tonkAuth) tonkAuth.destroy();
  } catch (error) {
    const duration = Date.now() - startTime;
    trackCommandError('server-create', error as Error, duration, {
      name: options.name,
      region: options.region,
    });

    console.error(
      chalk.red(
        `Error: ${error instanceof Error ? error.message : String(error)}`
      )
    );
    await shutdownAnalytics();
    if (tonkAuth) tonkAuth.destroy();
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
    config.deployment.defaultRegion
  )
  .option(
    '-m, --memory <memory>',
    'Memory allocation (e.g., 256mb, 1gb)',
    config.deployment.defaultMemory
  )
  .option('-c, --cpus <cpus>', 'Number of CPUs', config.deployment.defaultCpus)
  .option(
    '--remote',
    'Use remote Docker build (slower but works with limited local resources)'
  )
  .action(handleServerCreateCommand);

// Server ls command
const serverLsCommand = new Command('ls')
  .description('List bundles on a remote Tonk server')
  .option('-s, --server <server>', 'Name of the Tonk server')
  .action(async options => {
    const startTime = Date.now();
    let tonkAuth: any = null;

    try {
      // Run auth check (replaces the preAction hook)
      const authHook = await getAuthHook();
      await authHook();

      // Get TonkAuth instance for cleanup
      tonkAuth = await getTonkAuth();

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
      if (tonkAuth) tonkAuth.destroy();
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('server-ls', error as Error, duration, {
        server: options.server,
      });

      console.error(
        chalk.red(
          `Error: ${error instanceof Error ? error.message : String(error)}`
        )
      );
      await shutdownAnalytics();
      if (tonkAuth) tonkAuth.destroy();
      process.exitCode = 1;
    }
  });

// Server ps command
const serverPsCommand = new Command('ps')
  .description('List running bundles on a remote Tonk server')
  .option('-s, --server <server>', 'Name of the Tonk server')
  .action(async options => {
    const startTime = Date.now();
    let tonkAuth: any = null;

    try {
      // Run auth check (replaces the preAction hook)
      const authHook = await getAuthHook();
      await authHook();

      // Get TonkAuth instance for cleanup
      tonkAuth = await getTonkAuth();

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
      if (tonkAuth) tonkAuth.destroy();
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('server-ps', error as Error, duration, {
        server: options.server,
      });

      console.error(
        chalk.red(
          `Error: ${error instanceof Error ? error.message : String(error)}`
        )
      );
      await shutdownAnalytics();
      if (tonkAuth) tonkAuth.destroy();
      process.exitCode = 1;
    }
  });

// Server destroy command
const serverDestroyCommand = new Command('destroy')
  .description('Permanently delete a Tonk server and all its data')
  .action(async () => {
    const startTime = Date.now();
    let tonkAuth: any = null;

    try {
      // Run auth check
      const authHook = await getAuthHook();
      await authHook();

      // Get TonkAuth instance for cleanup
      tonkAuth = await getTonkAuth();

      trackCommand('server-destroy', {});

      console.log(chalk.red('⚠️  Server Destruction\n'));
      console.log(
        chalk.yellow(
          'WARNING: This will permanently delete your server and all data!'
        )
      );
      console.log(chalk.yellow('This action cannot be undone.\n'));

      // Check prerequisites
      await checkDeploymentService();

      // Fetch user's servers
      console.log(chalk.blue('Fetching your servers...'));
      let userServers: string[] = [];
      try {
        userServers = await fetchUserServers();
      } catch (error) {
        console.error(
          chalk.red(
            `Error fetching servers: ${error instanceof Error ? error.message : String(error)}`
          )
        );
        await shutdownAnalytics();
        process.exitCode = 1;
        return;
      }

      if (userServers.length === 0) {
        console.log(chalk.yellow('No servers found in your account.'));
        await shutdownAnalytics();
        return;
      }

      // Server selection
      let serverName: string;
      if (userServers.length === 1) {
        const { confirmSingle } = await inquirer.prompt([
          {
            type: 'confirm',
            name: 'confirmSingle',
            message: `Delete your only server '${userServers[0]}'?`,
            default: false,
          },
        ]);

        if (!confirmSingle) {
          console.log(chalk.yellow('Server deletion cancelled'));
          await shutdownAnalytics();
          return;
        }

        serverName = userServers[0];
      } else {
        const { selectedServer } = await inquirer.prompt([
          {
            type: 'list',
            name: 'selectedServer',
            message: `Select a server to PERMANENTLY DELETE (${userServers.length} available):`,
            choices: userServers,
          },
        ]);
        serverName = selectedServer;
      }

      // Final confirmation - require typing server name
      const { confirmName } = await inquirer.prompt([
        {
          type: 'input',
          name: 'confirmName',
          message: `Type the server name '${serverName}' to confirm deletion:`,
          validate: (input: string) => {
            if (input.trim() !== serverName) {
              return `Please type exactly '${serverName}' to confirm`;
            }
            return true;
          },
        },
      ]);

      if (confirmName.trim() !== serverName) {
        console.log(chalk.yellow('Server deletion cancelled'));
        await shutdownAnalytics();
        return;
      }

      // Delete the server
      console.log(chalk.red(`\nDeleting server '${serverName}'...`));
      await deleteServer(serverName);

      const duration = Date.now() - startTime;
      trackCommandSuccess('server-destroy', duration, {
        serverName,
      });
      await shutdownAnalytics();
      if (tonkAuth) tonkAuth.destroy();
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('server-destroy', error as Error, duration, {});

      console.error(
        chalk.red(
          `Error: ${error instanceof Error ? error.message : String(error)}`
        )
      );
      await shutdownAnalytics();
      if (tonkAuth) tonkAuth.destroy();
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
    let tonkAuth: any = null;

    try {
      // Run auth check (replaces the preAction hook)
      const authHook = await getAuthHook();
      await authHook();

      // Get TonkAuth instance for cleanup
      tonkAuth = await getTonkAuth();

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
      const { confirm } = await inquirer.prompt([
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
      if (tonkAuth) tonkAuth.destroy();
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('server-rm', error as Error, duration, {
        bundleName,
        server: options.server,
      });

      console.error(
        chalk.red(
          `Error: ${error instanceof Error ? error.message : String(error)}`
        )
      );
      await shutdownAnalytics();
      if (tonkAuth) tonkAuth.destroy();
      process.exitCode = 1;
    }
  });

// Main server command
export const serverCommand = new Command('server').description(
  'Manage Tonk servers'
);

serverCommand.addCommand(serverCreateCommand);
serverCommand.addCommand(serverLsCommand);
serverCommand.addCommand(serverPsCommand);
serverCommand.addCommand(serverRmCommand);
serverCommand.addCommand(serverDestroyCommand);

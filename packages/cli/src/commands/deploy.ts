import {Command} from 'commander';
import chalk from 'chalk';
import fs from 'fs-extra';
import path from 'path';
import {execSync} from 'child_process';
import inquirer from 'inquirer';
import ora from 'ora';
import tar from 'tar';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
} from '../utils/analytics.js';

interface DeployOptions {
  name?: string;
  server?: string;
  region?: string;
  memory?: string;
  cpus?: string;
  skipBuild?: boolean;
  remote?: boolean;
}

interface ServerOptions {
  name?: string;
  region?: string;
  memory?: string;
  cpus?: string;
  remote?: boolean;
}

interface TonkConfig {
  name?: string;
  version?: string;
  [key: string]: any;
}

const DEPLOYMENT_SERVICE_URL =
  process.env.TONK_DEPLOYMENT_SERVICE_URL ||
  'http://ec2-51-20-65-254.eu-north-1.compute.amazonaws.com:4444';
// 'http://localhost:4444';

/**
 * Checks if the deployment service is reachable
 */
async function checkDeploymentService(): Promise<void> {
  try {
    const response = await fetch(`${DEPLOYMENT_SERVICE_URL}/health`);
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
    process.exit(1);
  }
}

/**
 * Reads the tonk.config.json file to get project information
 */
function readTonkConfig(): TonkConfig | null {
  const configPath = path.join(process.cwd(), 'tonk.config.json');

  if (!fs.existsSync(configPath)) {
    return null;
  }

  try {
    return JSON.parse(fs.readFileSync(configPath, 'utf8'));
  } catch (error) {
    console.warn(chalk.yellow('Warning: Could not parse tonk.config.json'));
    return null;
  }
}

/**
 * Reads package.json to get project information
 */
function readPackageJson(): any {
  const packagePath = path.join(process.cwd(), 'package.json');

  if (!fs.existsSync(packagePath)) {
    throw new Error(
      'package.json not found. Are you in a Tonk project directory?',
    );
  }

  try {
    return JSON.parse(fs.readFileSync(packagePath, 'utf8'));
  } catch (error) {
    throw new Error('Could not parse package.json');
  }
}

/**
 * Determines the app name for deployment
 */
function determineAppName(
  options: DeployOptions,
  tonkConfig: TonkConfig | null,
  packageJson: any,
): string {
  if (options.name) {
    return options.name;
  }

  if (tonkConfig?.name) {
    return tonkConfig.name.replace(/[^a-zA-Z0-9-]/g, '-').toLowerCase();
  }

  if (packageJson?.name) {
    return packageJson.name
      .replace(/^@.*\//, '')
      .replace(/[^a-zA-Z0-9-]/g, '-')
      .toLowerCase();
  }

  return path
    .basename(process.cwd())
    .replace(/[^a-zA-Z0-9-]/g, '-')
    .toLowerCase();
}

/**
 * Creates an app bundle for deployment
 */
async function createAppBundle(): Promise<string> {
  const spinner = ora('Creating app bundle...').start();

  try {
    const bundlePath = path.join('/tmp', `tonk-app-${Date.now()}.tar.gz`);
    const cwd = process.cwd();

    // Create tar archive excluding node_modules, .git, etc.
    await tar.create(
      {
        file: bundlePath,
        cwd,
        gzip: true,
        filter: (filePath: string) => {
          const exclude = [
            'node_modules',
            '.git',
            '.env',
            '.next',
            'coverage',
            '*.log',
            '.DS_Store',
            'Thumbs.db',
          ];
          return !exclude.some(pattern => filePath.includes(pattern));
        },
      },
      ['.'],
    );

    spinner.succeed('App bundle created');
    return bundlePath;
  } catch (error) {
    spinner.fail('Failed to create app bundle');
    throw error;
  }
}

/**
 * Builds the Tonk app with the correct base path
 */
async function buildApp(skipBuild: boolean, bundleName: string): Promise<void> {
  if (skipBuild) {
    console.log(chalk.yellow('Skipping build step'));
    return;
  }

  const spinner = ora(`Building Tonk app...`).start();

  try {
    // Set the base path environment variable for Vite
    const env = {
      ...process.env,
      VITE_BASE_PATH: `/${bundleName}/`,
    };

    execSync('npm run build', {stdio: 'pipe', env});
    spinner.succeed('App built successfully');
  } catch (error) {
    spinner.fail('Build failed');
    throw new Error(
      'Failed to build app. Make sure you have a build script in package.json',
    );
  }
}

/**
 * Deploys a bundle to an existing Tonk server
 */
async function deployBundle(
  bundleName: string,
  serverName: string,
  tonkConfig: TonkConfig | null,
  packageJson: any,
): Promise<void> {
  const spinner = ora('Deploying...').start();

  try {
    // Create app bundle
    spinner.text = 'Creating app bundle...';
    const bundlePath = await createAppBundle();

    // Stop spinner before prompting for access code
    spinner.stop();

    // Prompt for access code
    const {accessCode} = await inquirer.prompt([
      {
        type: 'password',
        name: 'accessCode',
        message: 'Enter your access code:',
        mask: '*',
        validate: (input: string) => {
          if (!input || input.trim().length === 0) {
            return 'Access code is required';
          }
          return true;
        },
      },
    ]);

    // Restart spinner for upload
    spinner.start();

    // Prepare form data
    const formData = new FormData();
    const bundleBuffer = fs.readFileSync(bundlePath);
    const bundleBlob = new Blob([bundleBuffer], {type: 'application/gzip'});

    formData.append('appBundle', bundleBlob, 'app-bundle.tar.gz');

    formData.append(
      'deployData',
      JSON.stringify({
        bundleName,
        serverName,
        accessCode: accessCode.trim(),
        deployType: 'bundle',
        tonkConfig,
        packageJson,
      }),
    );

    // Send deployment request
    spinner.text = 'Uploading bundle to server...';
    const response = await fetch(`${DEPLOYMENT_SERVICE_URL}/deploy-bundle`, {
      method: 'POST',
      body: formData,
    });

    const result: any = await response.json();

    // Clean up bundle file
    fs.removeSync(bundlePath);

    if (!response.ok || !result.success) {
      throw new Error(result.error || 'Bundle deployment failed');
    }

    spinner.succeed('Bundle deployed successfully!');

    console.log(chalk.green(`🚀 Your Tonk bundle is deployed!`));
    console.log(chalk.blue(`   URL: ${result.bundleUrl}`));
  } catch (error) {
    spinner.fail('Deployment failed');
    throw error;
  }
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
    // Stop spinner before prompting for access code
    spinner.stop();

    // Prompt for access code
    const {accessCode} = await inquirer.prompt([
      {
        type: 'password',
        name: 'accessCode',
        message: 'Enter your access code:',
        mask: '*',
        validate: (input: string) => {
          if (!input || input.trim().length === 0) {
            return 'Access code is required';
          }
          return true;
        },
      },
    ]);

    // Restart spinner for server creation
    spinner.start();

    // Prepare form data for server creation
    const formData = new FormData();
    formData.append(
      'serverData',
      JSON.stringify({
        serverName,
        accessCode: accessCode.trim(),
        region: options.region || 'ord',
        memory: options.memory || '1gb',
        cpus: options.cpus || '1',
        remote: options.remote || false,
        deployType: 'server',
      }),
    );

    // Send server creation request
    spinner.text = 'Creating server...';
    const response = await fetch(`${DEPLOYMENT_SERVICE_URL}/create-server`, {
      method: 'POST',
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
 * Main deploy command handler
 */
async function handleDeployCommand(options: DeployOptions): Promise<void> {
  const startTime = Date.now();

  try {
    trackCommand('deploy', {
      name: options.name,
      server: options.server,
      region: options.region,
      memory: options.memory,
      cpus: options.cpus,
      skipBuild: options.skipBuild,
      remote: options.remote,
    });

    console.log(chalk.cyan('🚀 Tonk Bundle Deploy\n'));

    // Check prerequisites
    console.log(chalk.blue('Checking prerequisites...'));
    await checkDeploymentService();

    // Read project configuration
    const tonkConfig = readTonkConfig();
    const packageJson = readPackageJson();
    const bundleName = determineAppName(options, tonkConfig, packageJson);

    // Determine server name
    let serverName = options.server;
    if (!serverName) {
      const {inputServer} = await inquirer.prompt([
        {
          type: 'input',
          name: 'inputServer',
          message: 'Enter your Tonk server name:',
          validate: (input: string) => {
            if (!input || input.trim().length === 0) {
              return 'Server name is required';
            }
            return true;
          },
        },
      ]);
      serverName = inputServer.trim();
    }

    console.log(chalk.green(`✓ Bundle name: ${bundleName}`));
    console.log(chalk.green(`✓ Target server: ${serverName}`));

    // Confirm deployment
    if (!options.name) {
      const {confirm} = await inquirer.prompt([
        {
          type: 'confirm',
          name: 'confirm',
          message: `Deploy bundle "${bundleName}" to server "${serverName}"?`,
          default: true,
        },
      ]);

      if (!confirm) {
        console.log(chalk.yellow('Deployment cancelled'));
        return;
      }
    }

    // Build the app with the correct base path
    await buildApp(options.skipBuild || false, bundleName);

    // Deploy bundle to the Tonk server
    await deployBundle(bundleName, serverName!, tonkConfig, packageJson);

    const duration = Date.now() - startTime;
    trackCommandSuccess('deploy', duration, {
      bundleName,
      serverName,
      region: options.region,
      memory: options.memory,
      cpus: options.cpus,
      remote: options.remote,
    });
  } catch (error) {
    const duration = Date.now() - startTime;
    trackCommandError('deploy', error as Error, duration, {
      name: options.name,
      server: options.server,
      region: options.region,
    });

    console.error(
      chalk.red(
        `Error: ${error instanceof Error ? error.message : String(error)}`,
      ),
    );
    process.exit(1);
  }
}

/**
 * Server create command handler
 */
async function handleServerCreateCommand(
  options: ServerOptions,
): Promise<void> {
  const startTime = Date.now();

  try {
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
    process.exit(1);
  }
}

export const deployCommand = new Command('deploy')
  .description('Deploy a Tonk bundle to an existing server')
  .option(
    '-n, --name <name>',
    'Name for the deployed bundle (defaults to package.json name)',
  )
  .option('-s, --server <server>', 'Name of the Tonk server to deploy to')
  .option('-r, --region <region>', 'Region to deploy to', 'ord')
  .option(
    '-m, --memory <memory>',
    'Memory allocation (e.g., 256mb, 1gb)',
    '1gb',
  )
  .option('-c, --cpus <cpus>', 'Number of CPUs', '1')
  .option('--skip-build', 'Skip the build step')
  .option(
    '--remote',
    'Use remote Docker build (slower but works with limited local resources)',
  )
  .action(handleDeployCommand);

export const serverCommand = new Command('server').description(
  'Manage Tonk servers',
);

const serverCreateCommand = new Command('create')
  .description('Create a new Tonk server')
  .option('-n, --name <name>', 'Name for the server')
  .option('-r, --region <region>', 'Region to deploy to', 'ord')
  .option(
    '-m, --memory <memory>',
    'Memory allocation (e.g., 256mb, 1gb)',
    '1gb',
  )
  .option('-c, --cpus <cpus>', 'Number of CPUs', '1')
  .option(
    '--remote',
    'Use remote Docker build (slower but works with limited local resources)',
  )
  .action(handleServerCreateCommand);

serverCommand.addCommand(serverCreateCommand);

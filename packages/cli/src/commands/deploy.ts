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
  region?: string;
  memory?: string;
  cpus?: string;
  skipBuild?: boolean;
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
            'dist',
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
 * Builds the Tonk app if needed
 */
async function buildApp(skipBuild: boolean): Promise<void> {
  if (skipBuild) {
    console.log(chalk.yellow('Skipping build step'));
    return;
  }

  const spinner = ora('Building Tonk app...').start();

  try {
    execSync('npm run build', {stdio: 'pipe'});
    spinner.succeed('App built successfully');
  } catch (error) {
    spinner.fail('Build failed');
    throw new Error(
      'Failed to build app. Make sure you have a build script in package.json',
    );
  }
}

/**
 * Deploys the app using the Tonk deployment service
 */
async function deployApp(
  appName: string,
  options: DeployOptions,
  tonkConfig: TonkConfig | null,
  packageJson: any,
): Promise<void> {
  const spinner = ora('Deploying...').start();

  try {
    // Create app bundle
    spinner.text = 'Creating app bundle...';
    const bundlePath = await createAppBundle();

    // Prepare form data
    const formData = new FormData();
    const bundleBuffer = fs.readFileSync(bundlePath);
    const bundleBlob = new Blob([bundleBuffer], {type: 'application/gzip'});

    formData.append('appBundle', bundleBlob, 'app-bundle.tar.gz');
    formData.append(
      'deployData',
      JSON.stringify({
        appName,
        region: options.region || 'ord',
        memory: options.memory || '1gb',
        cpus: options.cpus || '1',
        remote: options.remote || false,
        tonkConfig,
        packageJson,
      }),
    );

    // Send deployment request
    spinner.text = 'Uploading to deployment service...';
    const response = await fetch(`${DEPLOYMENT_SERVICE_URL}/deploy`, {
      method: 'POST',
      body: formData,
    });

    const result: any = await response.json();

    // Clean up bundle file
    fs.removeSync(bundlePath);

    if (!response.ok || !result.success) {
      throw new Error(result.error || 'Deployment failed');
    }

    spinner.succeed('Deployment successful!');

    console.log(chalk.green(`🚀 Your Tonk app is deployed!`));
    console.log(chalk.blue(`   URL: ${result.appUrl}`));
  } catch (error) {
    spinner.fail('Deployment failed');
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
      region: options.region,
      memory: options.memory,
      cpus: options.cpus,
      skipBuild: options.skipBuild,
      remote: options.remote,
    });

    console.log(chalk.cyan('🚀 Tonk Deploy\n'));

    // Check prerequisites
    console.log(chalk.blue('Checking prerequisites...'));
    await checkDeploymentService();

    // Read project configuration
    const tonkConfig = readTonkConfig();
    const packageJson = readPackageJson();
    const appName = determineAppName(options, tonkConfig, packageJson);

    console.log(chalk.green(`✓ App name: ${appName}`));

    // Confirm deployment
    if (!options.name) {
      const {confirm} = await inquirer.prompt([
        {
          type: 'confirm',
          name: 'confirm',
          message: `Deploy "${appName}"?`,
          default: true,
        },
      ]);

      if (!confirm) {
        console.log(chalk.yellow('Deployment cancelled'));
        return;
      }
    }

    // Build the app
    await buildApp(options.skipBuild || false);

    // Deploy using the Tonk deployment service
    await deployApp(appName, options, tonkConfig, packageJson);

    const duration = Date.now() - startTime;
    trackCommandSuccess('deploy', duration, {
      appName,
      region: options.region,
      memory: options.memory,
      cpus: options.cpus,
      remote: options.remote,
    });
  } catch (error) {
    const duration = Date.now() - startTime;
    trackCommandError('deploy', error as Error, duration, {
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
  .description('Deploy a Tonk app with one-touch hosting')
  .option(
    '-n, --name <name>',
    'Name for the deployed app (defaults to package.json name)',
  )
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

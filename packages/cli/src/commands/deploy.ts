import {Command} from 'commander';
import chalk from 'chalk';
import fs from 'fs-extra';
import path from 'path';
import {execSync} from 'child_process';
import inquirer from 'inquirer';
import ora from 'ora';
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

/**
 * Checks if flyctl is installed and available
 */
function checkFlyctl(): void {
  try {
    execSync('flyctl version', {stdio: 'pipe'});
  } catch (error) {
    console.error(
      chalk.red('Error: flyctl is not installed or not available in PATH.'),
    );
    console.log(
      chalk.yellow(
        'Please install Fly CLI from: https://fly.io/docs/flyctl/install/',
      ),
    );
    console.log(chalk.yellow('Or run: curl -L https://fly.io/install.sh | sh'));
    process.exit(1);
  }
}

/**
 * Checks if user is authenticated with Fly.io
 */
function checkFlyAuth(): void {
  try {
    execSync('flyctl auth whoami', {stdio: 'pipe'});
  } catch (error) {
    console.error(chalk.red('Error: Not authenticated with Fly.io.'));
    console.log(chalk.yellow('Please run: flyctl auth login'));
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
 * Creates a startup script for the Tonk app
 */
function createStartupScript(): void {
  const scriptsDir = path.join(process.cwd(), 'scripts');
  const scriptPath = path.join(scriptsDir, 'start-with-bundle.sh');

  if (fs.existsSync(scriptPath)) {
    console.log(chalk.blue('Using existing scripts/start-with-bundle.sh'));
    return;
  }

  // Ensure scripts directory exists
  if (!fs.existsSync(scriptsDir)) {
    fs.mkdirSync(scriptsDir, {recursive: true});
  }

  console.log(chalk.blue('Creating startup script...'));

  const script = `#!/bin/sh

echo "Installing curl..."
apk add curl

# Ensure data directories exist and have proper permissions
echo "Setting up data directories..."
mkdir -p /data/tonk/stores
mkdir -p /data/tonk/bundles
chown -R app:app /data 2>/dev/null || true

echo "Starting Tonk server in background..."
tsx src/docker-start.ts &
SERVER_PID=$!

echo "Waiting for Tonk server to start..."
until curl --output /dev/null --silent --fail http://localhost:7777/ping; do
  printf "."
  sleep 1
done

echo "Tonk server is up!"

echo "Creating app bundle..."
cd /tmp && tar -czf app-bundle.tar.gz -C /tmp/app-bundle .

echo "Uploading app bundle to Tonk server..."
curl -X POST -F "bundle=@/tmp/app-bundle.tar.gz" -F "name=tonk-app" http://localhost:7777/upload-bundle

echo "Starting app bundle..."
curl -X POST -H "Content-Type: application/json" -d '{"bundleName":"tonk-app","port":8000}' http://localhost:7777/start

echo "Waiting for app bundle to be ready on port 8000..."
until curl --output /dev/null --silent --fail http://localhost:8000/; do
  printf "."
  sleep 1
done

echo "App started successfully and is responding on port 8000!"

# Keep the server running
wait $SERVER_PID`;

  fs.writeFileSync(scriptPath, script);
  console.log(chalk.green('✓ Created scripts/start-with-bundle.sh'));
}

/**
 * Creates a Dockerfile for the Tonk app if it doesn't exist
 */
function createDockerfile(): void {
  const dockerfilePath = path.join(process.cwd(), 'Dockerfile');

  if (fs.existsSync(dockerfilePath)) {
    console.log(chalk.blue('Using existing Dockerfile'));
    return;
  }

  console.log(chalk.blue('Creating Dockerfile for Tonk app...'));

  const dockerfile = `# Build stage
FROM node:20-alpine as build

WORKDIR /app

# Copy package files and install dependencies
COPY package.json ./
# Copy lock file if it exists (supports npm, yarn, pnpm)
COPY package-lock.json* yarn.lock* pnpm-lock.yaml* ./
# Use appropriate install command based on available lock files
RUN if [ -f "package-lock.json" ]; then npm ci; \\
  elif [ -f "yarn.lock" ]; then yarn install --frozen-lockfile; \\
  elif [ -f "pnpm-lock.yaml" ]; then npm install -g pnpm && pnpm install --frozen-lockfile; \\
  else npm install; \\
  fi

# Copy source files and build the app
COPY . .
RUN npm run build

# Production stage - Use tonklabs/tonk-server as base
FROM tonklabs/tonk-server:latest

WORKDIR /app

# Copy built app from build stage to tmp directory
COPY --from=build /app/dist /tmp/app-bundle

# Copy startup script
COPY scripts/start-with-bundle.sh /app/start-with-bundle.sh
RUN chmod +x /app/start-with-bundle.sh

# Expose both Tonk server port and app port
EXPOSE 7777 8000

# Health check on the app port
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \\
  CMD wget --no-verbose --tries=1 --spider http://localhost:8000/ || exit 1

# Use the startup script
CMD ["sh", "/app/start-with-bundle.sh"]
`;

  fs.writeFileSync(dockerfilePath, dockerfile);
  console.log(chalk.green('✓ Created Dockerfile'));
}

/**
 * Creates a fly.toml configuration file
 */
function createFlyConfig(appName: string, options: DeployOptions): void {
  const flyConfigPath = path.join(process.cwd(), 'fly.toml');

  if (fs.existsSync(flyConfigPath)) {
    console.log(chalk.blue('Using existing fly.toml'));
    return;
  }

  console.log(chalk.blue('Creating fly.toml configuration...'));

  const region = options.region || 'ord';

  const flyConfig = `# Fly.io configuration for Tonk app
app = "${appName}"
primary_region = "${region}"

[build]

# Main HTTP service for the frontend app
[http_service]
  internal_port = 8000
  force_https = true
  auto_stop_machines = true
  auto_start_machines = true

# Additional service for Tonk sync server
[[services]]
  internal_port = 7777
  protocol = "tcp"
  auto_stop_machines = "stop"
  auto_start_machines = true
  min_machines_running = 0

[[vm]]
  memory = "1gb"
  cpu_kind = "shared"
  cpus = 1

# Volume mounts for persistent data
[mounts]
  source = "tonk_data"
  destination = "/data"

[env]
  NODE_ENV = "production"
  PORT = "7777"
  # Configure Tonk server paths to use persistent volumes
  STORES_PATH = "/data/tonk/stores"
  BUNDLES_PATH = "/data/tonk/bundles"
`;

  fs.writeFileSync(flyConfigPath, flyConfig);
  console.log(chalk.green('✓ Created fly.toml'));
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
 * Gets the user's default organization
 */
function getDefaultOrg(): string {
  try {
    const orgsOutput = execSync('flyctl orgs list', {encoding: 'utf8'});
    const lines = orgsOutput.split('\n');

    // Skip header lines and empty lines, find the first data row
    for (const line of lines) {
      const trimmed = line.trim();

      // Skip empty lines, header line, and separator line
      if (
        !trimmed ||
        trimmed.includes('Name') ||
        trimmed.includes('----') ||
        trimmed.includes('Slug')
      ) {
        continue;
      }

      // Parse the line: Name, Slug, Type are space-separated
      const parts = trimmed.split(/\s+/);
      if (parts.length >= 3) {
        const slug = parts[1]; // The second column is the slug

        // Prefer personal org if available
        if (parts[2] === 'PERSONAL') {
          return slug;
        }

        // Otherwise, use the first valid org slug
        if (slug && slug !== 'Slug') {
          return slug;
        }
      }
    }

    throw new Error('No valid organizations found');
  } catch (error) {
    throw new Error(
      'Could not determine default organization. Please run "flyctl orgs list" to see available organizations.',
    );
  }
}

/**
 * Deploys the app to Fly.io
 */
async function deployToFly(
  appName: string,
  remote: boolean,
  region: string,
): Promise<void> {
  const spinner = ora('Deploying to Fly.io...').start();

  try {
    // Create the app if it doesn't exist
    try {
      execSync(`flyctl apps list | grep "^${appName}"`, {stdio: 'pipe'});
      spinner.text = 'App exists, deploying...';
    } catch (error) {
      spinner.text = 'Creating new Fly.io app...';
      const defaultOrg = getDefaultOrg();
      execSync(`flyctl apps create ${appName} --org ${defaultOrg}`, {
        stdio: 'pipe',
      });
    }

    // Create volumes if they don't exist
    try {
      execSync(`flyctl volumes list -a ${appName} | grep tonk_data`, {
        stdio: 'pipe',
      });
    } catch (error) {
      spinner.text = 'Creating tonk_data volume...';
      execSync(
        `flyctl volumes create tonk_data --size 5 -a ${appName} --region ${region} --yes`,
        {
          stdio: 'pipe',
        },
      );
    }

    // Deploy the app
    spinner.text = 'Deploying app...';
    const deployCmd = remote ? 'flyctl deploy --remote-only' : 'flyctl deploy';
    execSync(`${deployCmd} -a ${appName}`, {stdio: 'pipe'});

    spinner.succeed('Deployment successful!');

    // Get the app status and hostname
    const statusOutput = execSync(`flyctl status -a ${appName}`, {
      encoding: 'utf8',
    });

    // Extract hostname from status output
    const hostnameMatch = statusOutput.match(/Hostname\s*=\s*(.+)/);
    const hostname = hostnameMatch
      ? hostnameMatch[1].trim()
      : `${appName}.fly.dev`;

    console.log(chalk.green(`🚀 Your Tonk app is deployed!`));
    console.log(chalk.blue(`   URL: https://${hostname}`));
    console.log(chalk.blue(`   Dashboard: https://fly.io/apps/${appName}`));
    console.log(chalk.yellow(`   Logs: flyctl logs -a ${appName}`));
    console.log(chalk.yellow(`   SSH: flyctl ssh console -a ${appName}`));
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
    checkFlyctl();
    checkFlyAuth();

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
          message: `Deploy "${appName}" to Fly.io?`,
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

    // Create startup script, Docker and Fly configuration files
    createStartupScript();
    createDockerfile();
    createFlyConfig(appName, options);

    // Deploy to Fly.io
    await deployToFly(
      appName,
      options.remote || false,
      options.region || 'ord',
    );

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
  .description('Deploy a Tonk app to Fly.io with one-touch hosting')
  .option(
    '-n, --name <name>',
    'Name for the deployed app (defaults to package.json name)',
  )
  .option('-r, --region <region>', 'Fly.io region to deploy to', 'ord')
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

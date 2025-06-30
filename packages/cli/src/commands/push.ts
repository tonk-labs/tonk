import {Command} from 'commander';
import chalk from 'chalk';
import fs from 'fs-extra';
import path from 'path';
import fetch from 'node-fetch';
import {FormData} from 'formdata-node';
import {fileFromPath} from 'formdata-node/file-from-path';
import {FormDataEncoder} from 'form-data-encoder';
import {Readable} from 'stream';
import * as tar from 'tar';
import {spawn} from 'child_process';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
  shutdownAnalytics,
} from '../utils/analytics.js';
import {getServerConfig} from '../config/environment.js';

interface PushOptions {
  url: string;
  name?: string;
  dir?: string;
  noBuild?: boolean;
  noStart?: boolean;
  route?: string;
}

interface UploadResult {
  success: boolean;
  message: string;
  bundleName: string;
  hasServices?: boolean;
}

interface StartResult {
  id: string;
  bundleName: string;
  port?: number;
  route?: string;
  status: string;
  url?: string;
}

/**
 * Validates that the source directory exists
 */
async function validateSourceDirectory(sourcePath: string): Promise<void> {
  if (!fs.existsSync(sourcePath)) {
    console.error(chalk.red(`Error: Directory ${sourcePath} does not exist.`));
    console.log(
      chalk.yellow(`Run 'npm run build' to create the dist directory.`),
    );
    await shutdownAnalytics();
    process.exitCode = 1;
    return;
  }
}

/**
 * Determines the bundle name based on package.json or directory name
 */
function determineBundleName(
  projectRoot: string,
  providedName?: string,
): string {
  let defaultName;
  try {
    const packageJsonPath = path.join(projectRoot, 'package.json');
    if (fs.existsSync(packageJsonPath)) {
      const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'));
      defaultName = packageJson.name
        ? packageJson.name.replace(/^@.*\//, '')
        : undefined;
    }
  } catch (err) {
    // Ignore error, will fall back to directory name
  }

  if (!defaultName) {
    defaultName = path.basename(projectRoot);
  }

  // Use provided name or default
  return providedName || defaultName;
}

/**
 * Builds the project with the correct base path for route-based deployment
 */
async function buildWithBasePath(
  projectRoot: string,
  route: string,
): Promise<void> {
  return new Promise((resolve, reject) => {
    console.log(chalk.blue(`Building project with base path ${route}/...`));

    const buildCommand = 'npm';
    const buildArgs = ['run', 'build'];
    const env = {
      ...process.env,
      VITE_BASE_PATH: `${route}/`,
    };

    const buildProcess = spawn(buildCommand, buildArgs, {
      cwd: projectRoot,
      env,
      stdio: 'inherit',
    });

    buildProcess.on('close', code => {
      if (code === 0) {
        console.log(chalk.green('Build completed successfully!'));
        resolve();
      } else {
        reject(new Error(`Build failed with exit code ${code}`));
      }
    });

    buildProcess.on('error', error => {
      reject(new Error(`Failed to start build process: ${error.message}`));
    });
  });
}

/**
 * Creates a tarball of the source directory
 */
async function createTarball(
  sourcePath: string,
  projectRoot: string,
  bundleName: string,
): Promise<string> {
  console.log(
    chalk.blue(
      `Packaging contents of directory ${sourcePath} as bundle '${bundleName}'...`,
    ),
  );

  const tempTarPath = path.join(
    projectRoot,
    `${bundleName}-${Date.now()}.tar.gz`,
  );

  // Get all directories and files in the source directory
  const children = fs.readdirSync(sourcePath);

  if (children.length === 0) {
    throw new Error(`Source directory ${sourcePath} is empty.`);
  }

  // Create tarball with all children of the source directory
  await tar.create(
    {
      gzip: true,
      file: tempTarPath,
      cwd: sourcePath,
    },
    children,
  );

  return tempTarPath;
}

/**
 * Uploads the tarball to the Tonk server
 */
async function uploadBundle(
  serverUrl: string,
  bundleName: string,
  tarballPath: string,
): Promise<UploadResult> {
  console.log(chalk.blue(`Uploading bundle to ${serverUrl}...`));

  const form = new FormData();
  form.set('name', bundleName);
  form.set('bundle', await fileFromPath(tarballPath));

  const encoder = new FormDataEncoder(form);

  const response = await fetch(`${serverUrl}/upload-bundle`, {
    method: 'POST',
    headers: encoder.headers,
    body: Readable.from(encoder),
  });

  // Clean up temp file
  fs.unlinkSync(tarballPath);

  if (!response.ok) {
    const errorText = await response.text();
    throw new Error(`Server error: ${errorText}`);
  }

  const result = (await response.json()) as UploadResult;
  console.log(
    chalk.green(`Bundle '${result.bundleName}' uploaded successfully!`),
  );

  return result;
}

/**
 * Starts the bundle on the Tonk server
 */
async function startBundle(
  serverUrl: string,
  bundleName: string,
  route?: string,
): Promise<StartResult> {
  console.log(chalk.blue(`Starting bundle '${bundleName}'...`));

  const requestBody: {bundleName: string; route?: string} = {bundleName};

  // Use provided route or default to bundle name
  if (route) {
    requestBody.route = route;
  } else {
    requestBody.route = `/${bundleName}`;
  }

  const startResponse = await fetch(`${serverUrl}/start`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(requestBody),
  });

  if (!startResponse.ok) {
    const startErrorText = await startResponse.text();
    console.error(chalk.red(`Error starting bundle: ${startErrorText}`));
    throw new Error(`Error starting bundle: ${startErrorText}`);
  }

  const startResult = (await startResponse.json()) as StartResult;

  console.log(chalk.green(`Bundle started successfully!`));
  console.log(chalk.green(`Server ID: ${startResult.id}`));

  if (startResult.route) {
    console.log(chalk.green(`Route: ${startResult.route}`));
    console.log(
      chalk.green(
        `URL: ${startResult.url || `${serverUrl}${startResult.route}`}`,
      ),
    );
  } else if (startResult.port) {
    // Fallback for old port-based deployments
    console.log(chalk.green(`Running on port: ${startResult.port}`));
  }

  console.log(chalk.green(`Status: ${startResult.status}`));

  return startResult;
}

/**
 * Main push command handler
 */
async function handlePushCommand(options: PushOptions): Promise<void> {
  const startTime = Date.now();
  const serverUrl = options.url;
  const sourceDir = options.dir || './dist';
  const projectRoot = process.cwd();
  const sourcePath = path.isAbsolute(sourceDir)
    ? sourceDir
    : path.join(projectRoot, sourceDir);

  try {
    trackCommand('push', {
      serverUrl,
      sourceDir,
      name: options.name,
      noBuild: options.noBuild,
      noStart: options.noStart,
      route: options.route,
    });

    // Determine bundle name first (needed for route)
    const bundleName = determineBundleName(projectRoot, options.name);
    const route = options.route || `/${bundleName}`;

    // Build with correct base path by default (unless --no-build is specified)
    if (!options.noBuild) {
      await buildWithBasePath(projectRoot, route);
    }

    // Validate source directory
    await validateSourceDirectory(sourcePath);

    // Create tarball
    const tarballPath = await createTarball(
      sourcePath,
      projectRoot,
      bundleName,
    );

    // Upload bundle
    const uploadResult = await uploadBundle(serverUrl, bundleName, tarballPath);

    let startResult: StartResult | undefined;

    // Start bundle by default (unless --no-start is specified)
    if (!options.noStart) {
      startResult = await startBundle(
        serverUrl,
        uploadResult.bundleName,
        route,
      );
    } else {
      console.log(
        chalk.blue(
          `Use 'tonk start ${uploadResult.bundleName} --route ${route}' to start the bundle.`,
        ),
      );
    }

    const duration = Date.now() - startTime;
    trackCommandSuccess('push', duration, {
      bundleName: uploadResult.bundleName,
      serverUrl,
      started: !options.noStart,
      serverId: startResult?.id,
      route: startResult?.route,
      port: startResult?.port,
    });
    await shutdownAnalytics();
  } catch (error) {
    const duration = Date.now() - startTime;
    trackCommandError('push', error as Error, duration, {
      serverUrl,
      sourceDir,
      name: options.name,
      noBuild: options.noBuild,
      noStart: options.noStart,
      route: options.route,
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

export const pushCommand = new Command('push')
  .description('Package, upload, build and start a bundle on the Tonk server')
  .option(
    '-u, --url <url>',
    'URL of the Tonk server',
    getServerConfig().defaultUrl,
  )
  .option(
    '-n, --name <name>',
    'Name for the bundle (defaults to directory name)',
  )
  .option('-d, --dir <dir>', 'Directory to bundle (defaults to ./dist)')
  .option(
    '-r, --route <route>',
    'Route path for the bundle (defaults to /bundleName)',
  )
  .option('--no-build', 'Skip building the project before pushing')
  .option('--no-start', 'Skip starting the bundle after upload')
  .action(handlePushCommand);

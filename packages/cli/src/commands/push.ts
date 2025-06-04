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
import {trackCommand, trackCommandError, trackCommandSuccess} from '../utils/analytics.js';

interface PushOptions {
  url: string;
  name?: string;
  dir?: string;
  start?: boolean;
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
  port: number;
  status: string;
}

/**
 * Validates that the source directory exists
 */
function validateSourceDirectory(sourcePath: string): void {
  if (!fs.existsSync(sourcePath)) {
    console.error(chalk.red(`Error: Directory ${sourcePath} does not exist.`));
    console.log(
      chalk.yellow(
        `Run 'npm run build' or 'yarn build' to create the dist directory.`,
      ),
    );
    process.exit(1);
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
): Promise<StartResult> {
  console.log(chalk.blue(`Starting bundle '${bundleName}'...`));

  const startResponse = await fetch(`${serverUrl}/start`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({bundleName}),
  });

  if (!startResponse.ok) {
    const startErrorText = await startResponse.text();
    console.error(chalk.red(`Error starting bundle: ${startErrorText}`));
    throw new Error(`Error starting bundle: ${startErrorText}`);
  }

  const startResult = (await startResponse.json()) as StartResult;

  console.log(chalk.green(`Bundle server started successfully!`));
  console.log(chalk.green(`Server ID: ${startResult.id}`));
  console.log(chalk.green(`Running on port: ${startResult.port}`));
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
      start: options.start,
    });

    // Validate source directory
    validateSourceDirectory(sourcePath);

    // Determine bundle name
    const bundleName = determineBundleName(projectRoot, options.name);

    // Create tarball
    const tarballPath = await createTarball(
      sourcePath,
      projectRoot,
      bundleName,
    );

    // Upload bundle
    const uploadResult = await uploadBundle(serverUrl, bundleName, tarballPath);

    let startResult: StartResult | undefined;
    
    // Start bundle if requested
    if (options.start) {
      startResult = await startBundle(serverUrl, uploadResult.bundleName);
    } else {
      console.log(
        chalk.blue(
          `Use 'tonk start ${uploadResult.bundleName}' to start the bundle.`,
        ),
      );
    }

    const duration = Date.now() - startTime;
    trackCommandSuccess('push', duration, {
      bundleName: uploadResult.bundleName,
      serverUrl,
      started: options.start,
      serverId: startResult?.id,
      port: startResult?.port,
    });
  } catch (error) {
    const duration = Date.now() - startTime;
    trackCommandError('push', error as Error, duration, {
      serverUrl,
      sourceDir,
      name: options.name,
      start: options.start,
    });

    console.error(
      chalk.red(
        `Error: ${error instanceof Error ? error.message : String(error)}`,
      ),
    );
    process.exit(1);
  }
}

export const pushCommand = new Command('push')
  .description('Package and upload a bundle to the Tonk server')
  .option('-u, --url <url>', 'URL of the Tonk server', 'http://localhost:7777')
  .option(
    '-n, --name <name>',
    'Name for the bundle (defaults to directory name)',
  )
  .option('-d, --dir <dir>', 'Directory to bundle (defaults to ./dist)')
  .option('-s, --start', 'Start the bundle after upload')
  .action(handlePushCommand);

import {Command} from 'commander';
import path from 'path';
import fs from 'fs';
import {createServer} from '@tonk/server';
import chalk from 'chalk';

function normalizePath(inputPath: string): string {
  // If path starts with ~, expand to home directory
  if (inputPath.startsWith('~/')) {
    const homedir = process.env.HOME || process.env.USERPROFILE || '/home/node';
    return path.join(homedir, inputPath.substring(2));
  }

  // If path is an absolute path not in the user's home directory and not already expanded from ~,
  // redirect it to tonk-data
  if (
    inputPath.startsWith('/') &&
    !inputPath.startsWith(process.env.HOME || process.env.USERPROFILE || '') &&
    !inputPath.includes('node_modules')
  ) {
    const homedir = process.env.HOME || process.env.USERPROFILE || '/home/node';
    return path.join(homedir, 'tonk-data', inputPath);
  }

  return inputPath;
}

export const serveCommand = new Command('serve')
  .description('Start a Tonk app in production mode')
  .option('-p, --port <port>', 'Port to run the server on', '8080')
  .option('-d, --dist <path>', 'Path to the dist directory for single app mode')
  .option('-u, --userhub', 'Run in userhub mode')
  .option('-f, --filesystem <path>', 'Filesystem path for document storage')
  .action(async options => {
    const projectRoot = process.cwd();
    const port = parseInt(options.port, 10);
    const configFilePath = path.join(projectRoot, 'tonk.config.json');

    // Determine whether we're using single app mode or hub mode
    let distPath: string | undefined;

    if (options.userhub) {
      if (!options.filesystem) {
        console.error(
          chalk.red(`Error: cannot run in hub mode without a filesystem path`),
        );
        process.exit(1);
      }
      // do nothing
    } else if (options.dist) {
      // Single app mode: serve a single app from dist
      distPath = path.isAbsolute(options.dist)
        ? options.dist
        : path.join(projectRoot, options.dist);

      // Check if the dist directory exists
      if (!fs.existsSync(distPath!)) {
        console.error(
          chalk.red(
            `Error: Dist directory not found at ${distPath}. Make sure you've built the project first.`,
          ),
        );
        console.log(
          chalk.yellow(
            `Tip: Run 'npm run build' or 'yarn build' before serving.`,
          ),
        );
        process.exit(1);
      }

      // Check if dist/index.html exists
      const indexPath = path.join(distPath!, 'index.html');
      if (!fs.existsSync(indexPath)) {
        console.error(
          chalk.red(
            `Error: index.html not found in ${distPath}. The build output may be incomplete.`,
          ),
        );
        process.exit(1);
      }
    } else {
      // Neither hub nor dist specified, default to 'dist' in current directory
      distPath = path.join(projectRoot, 'dist');

      if (!fs.existsSync(distPath)) {
        console.error(
          chalk.red(
            `Error: No --dist or --hub option provided, and default dist directory not found at ${distPath}.`,
          ),
        );
        process.exit(1);
      }

      // Check if dist/index.html exists
      const indexPath = path.join(distPath, 'index.html');
      if (!fs.existsSync(indexPath)) {
        console.error(
          chalk.red(
            `Error: index.html not found in ${distPath}. The build output may be incomplete.`,
          ),
        );
        process.exit(1);
      }
    }

    // Check for Backblaze and Filesystem configurations
    let backblazeConfig = null;
    let filesystemConfig = null;
    let primaryStorage = null;

    // Check if filesystem path was provided via command line
    const hasFilesystemFlag = !!options.filesystem;

    if (fs.existsSync(configFilePath)) {
      try {
        const configData = JSON.parse(fs.readFileSync(configFilePath, 'utf8'));

        // If backblaze is configured and enabled in the config file
        if (configData.backblaze && configData.backblaze.enabled) {
          backblazeConfig = {
            enabled: true,
            applicationKeyId: configData.backblaze.applicationKeyId,
            applicationKey: configData.backblaze.applicationKey,
            bucketId: configData.backblaze.bucketId,
            bucketName: configData.backblaze.bucketName,
            syncInterval: configData.backblaze.syncInterval || 300000, // 5 minutes default
            maxRetries: configData.backblaze.maxRetries || 3,
          };

          console.log(chalk.blue('Backblaze B2 backup enabled from config'));
        }

        // If filesystem storage is configured and enabled in the config file
        // and no command line path was provided
        if (
          !hasFilesystemFlag &&
          configData.filesystem &&
          configData.filesystem.enabled
        ) {
          let storagePath = configData.filesystem.storagePath || '~/tonk-data';
          storagePath = normalizePath(storagePath);

          filesystemConfig = {
            enabled: true,
            storagePath,
            syncInterval: configData.filesystem.syncInterval || 30000, // 30 seconds default
            createIfMissing: configData.filesystem.createIfMissing !== false, // default to true
          };

          console.log(chalk.blue('Filesystem storage enabled from config'));
        }

        // Check for primary storage configuration
        if (
          configData.primaryStorage &&
          (configData.primaryStorage === 'backblaze' ||
            configData.primaryStorage === 'filesystem')
        ) {
          primaryStorage = configData.primaryStorage;
          console.log(
            chalk.blue(`Primary storage set to ${primaryStorage} from config`),
          );
        }
      } catch (error) {
        console.warn(
          chalk.yellow(
            `Warning: Could not parse config file at ${configFilePath}. Storage backups will not be enabled.`,
          ),
        );
        console.warn(chalk.yellow(`Error details: ${error}`));
      }
    }

    // If filesystem path was provided via command line, override config settings
    if (hasFilesystemFlag) {
      const storagePath = normalizePath(options.filesystem);

      filesystemConfig = {
        enabled: true,
        storagePath,
        syncInterval: 30000, // 30 seconds default
        createIfMissing: true, // default to true
      };

      console.log(
        chalk.blue(`Filesystem storage enabled with path: ${storagePath}`),
      );

      // If no primary storage set and we have backblaze, set filesystem as primary
      if (!primaryStorage && backblazeConfig) {
        primaryStorage = 'filesystem';
        console.log(
          chalk.blue(
            'Primary storage set to filesystem from command line argument',
          ),
        );
      }
    }

    console.log(chalk.blue('Starting Tonk production server...'));

    try {
      // Start the production server with storage configs if available
      const serverConfig: any = {
        port,
        mode: 'production',
        verbose: true,
      };

      if (distPath) {
        serverConfig.distPath = distPath;
      }

      // Add Backblaze configuration if available
      if (backblazeConfig) {
        serverConfig.storage = {
          backblaze: backblazeConfig,
        };
      }

      // Add Filesystem configuration if available
      if (filesystemConfig) {
        serverConfig.filesystemStorage = filesystemConfig;
      }

      // Set primary storage if both are configured
      if (primaryStorage && backblazeConfig && filesystemConfig) {
        serverConfig.primaryStorage = primaryStorage;
      }

      const server = await createServer(serverConfig);

      // Log success info
      console.log(chalk.green(`Server running at http://localhost:${port}`));

      if (distPath) {
        console.log(chalk.green(`Serving single app from: ${distPath}`));
      }

      if (backblazeConfig) {
        console.log(
          chalk.green(`
Backblaze B2 backup is enabled for your Automerge documents.
Documents will be synced to your B2 bucket: ${backblazeConfig.bucketName}
          `),
        );
      }

      if (filesystemConfig) {
        console.log(
          chalk.green(`
Filesystem storage is enabled for your Automerge documents.
Documents will be stored at: ${filesystemConfig.storagePath}
          `),
        );
      }

      if (primaryStorage && backblazeConfig && filesystemConfig) {
        console.log(
          chalk.green(`
Primary storage is set to: ${primaryStorage}
          `),
        );
      }

      console.log(chalk.blue('Press Ctrl+C to stop the server'));

      // Handle process termination
      const cleanup = () => {
        console.log(chalk.yellow('\nShutting down server...'));
        server
          .stop()
          .then(() => {
            console.log(chalk.green('Server shutdown complete.'));
            // Force exit after brief delay to ensure any pending operations complete
            setTimeout(() => process.exit(0), 2000);
          })
          .catch(err => {
            console.error(chalk.red('Error during shutdown:'), err);
            // Force exit even on error
            process.exit(1);
          });
      };

      process.on('SIGINT', cleanup);
      process.on('SIGTERM', cleanup);
    } catch (error) {
      console.error(chalk.red('Failed to start server:'), error);
      process.exit(1);
    }
  });

import {Command} from 'commander';
import path from 'path';
import fs from 'fs';
import {createServer} from '@tonk/server';
import chalk from 'chalk';

export const serveCommand = new Command('serve')
  .description('Start a Tonk app in production mode')
  .option('-p, --port <port>', 'Port to run the server on', '8080')
  .option('-d, --dist <path>', 'Path to the dist directory', 'dist')
  .action(async options => {
    const projectRoot = process.cwd();
    const port = parseInt(options.port, 10);
    const distPath = path.isAbsolute(options.dist)
      ? options.dist
      : path.join(projectRoot, options.dist);
    const configFilePath = path.join(projectRoot, 'tonk.config.json');

    // Check if the dist directory exists
    if (!fs.existsSync(distPath)) {
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
    const indexPath = path.join(distPath, 'index.html');
    if (!fs.existsSync(indexPath)) {
      console.error(
        chalk.red(
          `Error: index.html not found in ${distPath}. The build output may be incomplete.`,
        ),
      );
      process.exit(1);
    }

    // Check for Backblaze and Filesystem configurations
    let backblazeConfig = null;
    let filesystemConfig = null;
    let primaryStorage = null;

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
        if (configData.filesystem && configData.filesystem.enabled) {
          filesystemConfig = {
            enabled: true,
            storagePath: configData.filesystem.storagePath || '/data/tonk',
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

    console.log(chalk.blue('Starting Tonk production server...'));

    try {
      // Start the production server with storage configs if available
      const serverConfig: any = {
        port,
        mode: 'production',
        distPath,
        verbose: true,
      };

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
        server.stop().catch(console.error);
      };

      process.on('SIGINT', cleanup);
      process.on('SIGTERM', cleanup);
    } catch (error) {
      console.error(chalk.red('Failed to start server:'), error);
      process.exit(1);
    }
  });

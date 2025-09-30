import { createServer, type ServerOptions, type TonkServer } from './index.js';
import fs from 'node:fs';
import chalk from 'chalk';

const startDockerizedServer = async (): Promise<void> => {
  try {
    // Read configuration from environment variables
    const port = 7777;
    const persistencePath = process.env.PERSISTENCE_PATH || '/data/tonk';

    // Derive all paths from the persistence path
    const bundlesPath = `${persistencePath}/bundles`;
    const storesPath = `${persistencePath}/stores`;
    const configPath = `${persistencePath}/root.json`;

    // Ensure the persistence directory exists
    if (!fs.existsSync(persistencePath)) {
      fs.mkdirSync(persistencePath, { recursive: true });
      console.log(
        chalk.blue(`Created persistence directory: ${persistencePath}`)
      );
    }

    // Ensure subdirectories exist
    for (const dir of [bundlesPath, storesPath]) {
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
        console.log(chalk.blue(`Created directory: ${dir}`));
      }
    }

    console.log(chalk.cyan('📦 Tonk Docker Configuration:'));
    console.log(chalk.cyan('   Port:'), chalk.bold.green(port));
    console.log(chalk.cyan('   Bundles path:'), chalk.bold.green(bundlesPath));
    console.log(chalk.cyan('   Stores path:'), chalk.bold.green(storesPath));
    console.log(
      chalk.cyan('   Persistence path:'),
      chalk.bold.green(persistencePath)
    );
    console.log(chalk.cyan('   Config path:'), chalk.bold.green(configPath));

    // Create server configuration
    const serverConfig: ServerOptions = {
      persistencePath,
    };

    console.log(chalk.blue('Starting TonkServer in Docker...'));

    // Start the server
    const server: TonkServer = await createServer(serverConfig);

    // Handle process termination
    const cleanup = async (): Promise<void> => {
      console.log(chalk.yellow('\nShutting down server...'));
      try {
        await server.stop();
        console.log(chalk.green('Server shutdown complete.'));
        // Force exit after brief delay to ensure any pending operations complete
        setTimeout(() => process.exit(0), 500);
      } catch (err) {
        console.error(chalk.red('Error during shutdown:'), err);
        // Force exit even on error
        process.exit(1);
      }
    };

    process.on('SIGINT', cleanup);
    process.on('SIGTERM', cleanup);
  } catch (error) {
    console.error(chalk.red('Failed to start TonkServer in Docker:'), error);
    process.exit(1);
  }
};

startDockerizedServer();

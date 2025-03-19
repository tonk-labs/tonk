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

    console.log(chalk.blue('Starting Tonk production server...'));

    try {
      // Start the production server
      const server = await createServer({
        port,
        mode: 'production',
        distPath,
        verbose: true,
      });

      // Log success info
      console.log(chalk.green(`Server running at http://localhost:${port}`));
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

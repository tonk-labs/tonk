import {Command} from 'commander';
import path from 'path';
import {spawn} from 'child_process';
import {createServer} from '@tonk/server';
import chalk from 'chalk';
import fs from 'fs';

export const devCommand = new Command('dev')
  .description('Start a Tonk app in development mode')
  .option('-p, --port <port>', 'Port to run the sync server on', '4080')
  .option(
    '-f, --frontend-port <port>',
    'Port to run the frontend dev server on',
    '3000',
  )
  .option('--open', 'Enable auto-opening the browser')
  .action(async options => {
    const projectRoot = process.cwd();
    const port = parseInt(options.port, 10);
    const frontendPort = parseInt(options.frontendPort, 10);

    // Ensure we have a package.json
    const packageJsonPath = path.join(projectRoot, 'package.json');
    if (!fs.existsSync(packageJsonPath)) {
      console.error(
        chalk.red(
          'Error: package.json not found. Make sure you are in a Tonk project directory.',
        ),
      );
      process.exit(1);
    }

    console.log(chalk.blue('Starting Tonk development environment...'));

    // Start the sync server
    const server = await createServer({
      port,
      mode: 'development',
      verbose: true,
      distPath: undefined,
    });

    // Start webpack dev server
    const webpackArgs = [
      'webpack',
      'serve',
      '--mode',
      'development',
      '--port',
      frontendPort.toString(),
    ];

    // Add --open flag if needed
    if (options.open) {
      webpackArgs.push('--open');
    }

    console.log(
      chalk.blue(`Starting webpack dev server on port ${frontendPort}...`),
    );
    console.log(chalk.blue(`Sync server running on port ${port}`));

    const webpackProcess = spawn('npx', webpackArgs, {
      cwd: projectRoot,
      stdio: 'inherit',
      shell: true,
      env: {
        ...process.env,
        // PORT: frontendPort.toString(),
        // We now assume server is running on 8080 at all times with the hub
        PORT: '8080',
      },
    });

    // Handle process termination
    const cleanup = () => {
      console.log(chalk.yellow('\nShutting down servers...'));
      webpackProcess.kill();
      server.stop().catch(console.error);
    };

    process.on('SIGINT', cleanup);
    process.on('SIGTERM', cleanup);

    // Handle webpack process exit
    webpackProcess.on('exit', code => {
      if (code !== 0 && code !== null) {
        console.error(chalk.red(`Webpack process exited with code ${code}`));
      }
      server.stop().catch(console.error);
      process.exit(code || 0);
    });
  });

import {Command} from 'commander';
import path from 'path';
import {spawn, ChildProcess} from 'child_process';
import chalk from 'chalk';
import fs from 'fs';
import {fileURLToPath} from 'url';

// Get the directory name in ES modules
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export const guiCommand = new Command('gui')
  .description('Launch the Tonk GUI application')
  .action(() => {
    console.log(chalk.blue('Launching Tonk GUI...'));
    launchTonkGUI();
  });

function launchTonkGUI() {
  // Determine the paths - CLI is in src/commands, so we need to go up to the project root
  const projectRoot = path.resolve(__dirname, '../../..');
  const electronAppPath = path.resolve(projectRoot, 'hub');
  const hubUIPath = path.resolve(electronAppPath, 'hub-ui');

  // Check if the Electron app exists
  if (!fs.existsSync(electronAppPath)) {
    console.error(
      chalk.red(
        `Electron app not found at ${electronAppPath}. Please check your installation.`,
      ),
    );
    process.exit(1);
  }

  // Check if the hub UI exists
  if (!fs.existsSync(hubUIPath)) {
    console.error(
      chalk.red(
        `Hub UI not found at ${hubUIPath}. Please check your installation.`,
      ),
    );
    process.exit(1);
  }

  try {
    // Start the webpack development server for the hub UI
    console.log(chalk.blue('Starting Hub UI development server...'));
    const uiProcess = startUIServer(hubUIPath);

    // Give the UI server a moment to start
    setTimeout(() => {
      // Launch the Electron app
      console.log(chalk.blue('Starting Electron app...'));
      const electronProcess = startElectronApp(electronAppPath, uiProcess);

      // Handle process termination
      process.on('SIGINT', () => {
        console.log(chalk.yellow('\nShutting down Tonk GUI...'));
        electronProcess.kill();
        uiProcess.kill();
        process.exit(0);
      });
    }, 2000); // Wait 2 seconds before starting Electron
  } catch (error) {
    console.error(
      chalk.red(`Error launching Tonk GUI: ${(error as Error).message}`),
    );
    process.exit(1);
  }
}

function startUIServer(hubUIPath: string): ChildProcess {
  console.log(chalk.blue(`Starting webpack dev server in ${hubUIPath}...`));

  const uiProcess = spawn('npm', ['run', 'start'], {
    cwd: hubUIPath,
    stdio: 'pipe',
    shell: true,
  });

  uiProcess.stdout.on('data', data => {
    const output = data.toString();
    console.log(chalk.green(output.trim()));
  });

  uiProcess.stderr.on('data', data => {
    const output = data.toString();
    console.error(chalk.yellow(output.trim()));
  });

  uiProcess.on('error', err => {
    console.error(chalk.red(`Failed to start UI server: ${err.message}`));
  });

  uiProcess.on('close', code => {
    if (code !== 0) {
      console.log(chalk.yellow(`UI server exited with code ${code}`));
    }
  });

  return uiProcess;
}

function startElectronApp(
  electronAppPath: string,
  uiProcess: ChildProcess,
): ChildProcess {
  console.log(chalk.blue(`Starting Electron app in ${electronAppPath}...`));

  // Check if package.json exists in the electron app directory
  const packageJsonPath = path.join(electronAppPath, 'package.json');
  if (!fs.existsSync(packageJsonPath)) {
    console.error(chalk.red(`No package.json found in ${electronAppPath}`));
    process.exit(1);
  }

  // Use npm run dev instead of npx electron .
  const electronProcess = spawn('npm', ['run', 'dev'], {
    cwd: electronAppPath,
    stdio: 'pipe',
    shell: true,
    env: {
      ...process.env,
      ELECTRON_START_URL: 'http://localhost:3333',
      // Enable Node.js integration in Electron
      NODE_ENV: 'development',
    },
  });

  electronProcess.stdout.on('data', data => {
    const output = data.toString();
    console.log(chalk.blue(output.trim()));
  });

  electronProcess.stderr.on('data', data => {
    const output = data.toString();
    console.error(chalk.red(output.trim()));
  });

  electronProcess.on('error', err => {
    console.error(chalk.red(`Failed to start Electron app: ${err.message}`));
  });

  electronProcess.on('close', code => {
    if (code !== 0) {
      console.log(chalk.yellow(`Electron app exited with code ${code}`));
    }
    console.log(
      chalk.yellow('Electron app closed, shutting down UI server...'),
    );
    uiProcess.kill(); // Kill the UI server when Electron closes
    console.log(chalk.yellow('UI server terminated.'));
    process.exit(0); // Exit when Electron closes
  });

  return electronProcess;
}

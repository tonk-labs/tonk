import chalk from 'chalk';
import {ChildProcess, spawn} from 'child_process';
import fs from 'fs';
import path from 'path';
import {fileURLToPath} from 'url';

export function launchTonkGUI() {
  // Use import.meta.url and fileURLToPath instead of __dirname
  const __filename = fileURLToPath(import.meta.url);
  const __dirname = path.dirname(__filename);

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
    throw new Error(
      `Electron app not found at ${electronAppPath}. Please check your installation.`,
    );
  }

  // Check if the hub UI exists
  if (!fs.existsSync(hubUIPath)) {
    console.error(
      chalk.red(
        `Hub UI not found at ${hubUIPath}. Please check your installation.`,
      ),
    );
    throw new Error(
      `Hub UI not found at ${hubUIPath}. Please check your installation.`,
    );
  }

  let uiProcess: ChildProcess | null = null;
  let electronProcess: ChildProcess | null = null;

  // Function to cleanly terminate both processes
  const cleanupProcesses = () => {
    console.log(chalk.yellow('\nShutting down Tonk GUI...'));

    if (electronProcess) {
      try {
        electronProcess.kill('SIGTERM');
        electronProcess = null;
      } catch (err) {
        console.error(
          chalk.red(
            `Error terminating Electron process: ${(err as Error).message}`,
          ),
        );
      }
    }

    if (uiProcess) {
      try {
        // First try a graceful termination
        uiProcess.kill('SIGTERM');

        // Force kill after a short timeout if still running
        setTimeout(() => {
          if (uiProcess) {
            try {
              // On macOS, we need SIGKILL to ensure termination
              uiProcess.kill('SIGKILL');
              uiProcess = null;
              console.log(chalk.yellow('Force-terminated UI process'));
            } catch (killErr) {
              console.error(
                chalk.red(
                  `Failed to force-kill UI process: ${
                    (killErr as Error).message
                  }`,
                ),
              );
            }
          }
        }, 1000);
      } catch (err) {
        console.error(
          chalk.red(`Error terminating UI process: ${(err as Error).message}`),
        );

        // Try force kill if normal termination failed
        try {
          uiProcess.kill('SIGKILL');
          uiProcess = null;
        } catch (killErr) {
          console.error(
            chalk.red(
              `Failed to force-kill UI process: ${(killErr as Error).message}`,
            ),
          );
        }
      }
    }

    // Also try to kill any process that might be using port 3333
    try {
      // For macOS/Linux
      const killCommand =
        process.platform === 'win32'
          ? spawn('cmd', [
              '/c',
              'FOR /F "tokens=5" %P IN (\'netstat -ano ^| findstr :3333 ^| findstr LISTENING\') DO taskkill /F /PID %P',
            ])
          : spawn('sh', ['-c', 'lsof -ti:3333 | xargs kill -9']);

      killCommand.on('error', err => {
        console.error(
          chalk.red(`Failed to kill process on port 3333: ${err.message}`),
        );
      });
    } catch (err) {
      console.error(
        chalk.red(
          `Error killing process on port 3333: ${(err as Error).message}`,
        ),
      );
    }

    console.log(chalk.yellow('Tonk GUI shutdown complete.'));
  };

  try {
    // Start the webpack development server for the hub UI
    console.log(chalk.blue('Starting Hub UI development server...'));
    uiProcess = startUIServer(hubUIPath);

    // Give the UI server a moment to start
    setTimeout(() => {
      // Launch the Electron app
      console.log(chalk.blue('Starting Electron app...'));
      electronProcess = startElectronApp(electronAppPath, cleanupProcesses);

      // Handle process termination
      process.on('SIGINT', cleanupProcesses);
      process.on('SIGTERM', cleanupProcesses);
      process.on('exit', cleanupProcesses);
    }, 2000); // Wait 2 seconds before starting Electron
  } catch (error) {
    console.error(
      chalk.red(`Error launching Tonk GUI: ${(error as Error).message}`),
    );
    cleanupProcesses();
    throw new Error(`Error launching Tonk GUI: ${(error as Error).message}`);
  }
}

function startUIServer(hubUIPath: string): ChildProcess {
  console.log(chalk.blue(`Starting webpack dev server in ${hubUIPath}...`));

  const uiProcess = spawn('pnpm', ['run', 'start'], {
    cwd: hubUIPath,
    stdio: 'pipe',
    shell: true,
    detached: false, // Ensure process is not detached from parent
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
    if (code !== 0 && code !== null) {
      console.log(chalk.yellow(`UI server exited with code ${code}`));
    }
  });

  return uiProcess;
}

function startElectronApp(
  electronAppPath: string,
  cleanupCallback: () => void,
): ChildProcess {
  console.log(chalk.blue(`Starting Electron app in ${electronAppPath}...`));

  // Check if package.json exists in the electron app directory
  const packageJsonPath = path.join(electronAppPath, 'package.json');
  if (!fs.existsSync(packageJsonPath)) {
    console.error(chalk.red(`No package.json found in ${electronAppPath}`));
    throw new Error(`No package.json found in ${electronAppPath}`);
  }

  // Use npm run dev instead of npx electron .
  const electronProcess = spawn('pnpm', ['run', 'dev'], {
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
    if (code !== 0 && code !== null) {
      console.log(chalk.yellow(`Electron app exited with code ${code}`));
    }
    console.log(
      chalk.yellow('Electron app closed, shutting down UI server...'),
    );
    // Call the cleanup function when Electron exits
    cleanupCallback();
  });

  return electronProcess;
}

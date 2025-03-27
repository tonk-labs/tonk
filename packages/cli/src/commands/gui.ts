import {Command} from 'commander';
import path from 'path';
import {spawn} from 'child_process';
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
    launchElectronApp();
  });

function launchElectronApp() {
  // Determine the path to the Electron app
  const electronAppPath = path.resolve(__dirname, '../../../hub/');

  // Check if the Electron app exists
  if (!fs.existsSync(electronAppPath)) {
    console.error(
      chalk.red('Electron app not found. Please install it first.'),
    );
    console.log(chalk.yellow('Run: npm install -g @tonkjs/electron-app'));
    process.exit(1);
  }

  try {
    // Launch the Electron app
    const electronProcess = spawn('npx', ['electron', '.'], {
      cwd: electronAppPath,
      stdio: 'inherit',
      shell: true,
    });

    electronProcess.on('error', err => {
      console.error(chalk.red(`Failed to start Electron app: ${err.message}`));
      process.exit(1);
    });

    electronProcess.on('close', code => {
      if (code !== 0) {
        console.log(chalk.yellow(`Electron app exited with code ${code}`));
      }
    });
  } catch (error) {
    console.error(
      chalk.red(`Error launching Electron app: ${(error as Error).message}`),
    );
    process.exit(1);
  }
}

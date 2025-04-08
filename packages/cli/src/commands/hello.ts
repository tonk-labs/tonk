import chalk from 'chalk';
import {Command} from 'commander';
import {launchTonkGUI} from '../lib/launchGUI.js';

export const helloCommand = new Command('hello')
  .description('Launch the Tonk HUB')
  .action(() => {
    console.log(chalk.blue('Launching Tonk GUI...'));
    launchTonkGUI();
  });

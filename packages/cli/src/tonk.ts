import {Command} from 'commander';
import {createCommand} from './commands/create.js';
import {devCommand} from './commands/dev.js';
import {serveCommand} from './commands/serve.js';
import {deployCommand} from './commands/deploy.js';
import {configCommand} from './commands/config.js';
import {helloCommand} from './commands/hello.js';
import {guiCommand} from './commands/gui.js';

const program = new Command();

// Main program setup
program
  .name('tonk')
  .description('The tonk cli helps you to manage your tonk stack and apps.')
  .version('0.1.0')
  .on('--help', () => {
    console.log('\nWork in progress!');
  });

program.addCommand(createCommand);
program.addCommand(devCommand);
program.addCommand(serveCommand);
program.addCommand(deployCommand);
program.addCommand(configCommand);
program.addCommand(helloCommand);
program.addCommand(guiCommand);

// Parse arguments
program.parse();

// Launch Electron app if no arguments provided
if (!process.argv.slice(2).length) {
  guiCommand.parse(process.argv);
}

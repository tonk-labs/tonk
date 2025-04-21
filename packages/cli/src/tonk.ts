import {Command} from 'commander';
import {createCommand} from './commands/create.js';
import {devCommand} from './commands/dev.js';
import {helloCommand} from './commands/hello.js';
import {printMarkdown} from './commands/prettyPrint.js';
import {serveCommand} from './commands/serve.js';
import {initCommand} from './commands/init.js';

const program = new Command();

// Main program setup
program
  .name('tonk')
  .description('The tonk cli helps you to manage your tonk stack and apps.')
  .version('0.1.2')
  .on('--help', () => {
    console.log('\nWork in progress!');
  });

program.addCommand(initCommand);
program.addCommand(createCommand);
program.addCommand(devCommand);
program.addCommand(serveCommand);
program.addCommand(helloCommand);
program.addCommand(printMarkdown);

program.parse();

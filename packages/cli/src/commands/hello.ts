import {Command} from 'commander';
import displayTonkAnimation from './hello/index.js';

export const helloCommand = new Command('hello')
  .description('Print out initial instructions and contact information')
  .action(() => {
    displayTonkAnimation();
  });

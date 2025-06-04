import {Command} from 'commander';
import child_process from 'child_process';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
} from '../utils/analytics.js';

export const createCommand = new Command('create')
  .description('Create a new tonk application or component')
  .option('-i, --init', 'initialize in the folder')
  .action(options => {
    const startTime = Date.now();

    try {
      trackCommand('create', {init: options.init});

      // Build the command with any passed options
      const createCommand = `npx @tonk/create ${options.init ? '-i' : ''}`;

      console.log(createCommand);

      // Execute the create package
      child_process.execSync(createCommand, {
        stdio: 'inherit',
        env: {...process.env},
      });

      const duration = Date.now() - startTime;
      trackCommandSuccess('create', duration, {init: options.init});

      process.exit(0);
    } catch (error: any) {
      const duration = Date.now() - startTime;
      trackCommandError('create', error, duration, {init: options.init});

      console.error('Failed to generate Tonk code:', error);
      process.exit(1);
    }
  });

import {Command} from 'commander';
import child_process from 'child_process';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
  shutdownAnalytics,
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
      trackCommandSuccess('create', duration, {
        init: options.init,
        command: createCommand,
      });

      shutdownAnalytics();
      process.exit(0);
    } catch (error: any) {
      const duration = Date.now() - startTime;

      // Enhanced error context
      let errorType = 'unknown_error';
      if (error instanceof Error) {
        if (error.message.includes('ENOENT')) {
          errorType = 'command_not_found';
        } else if (error.message.includes('EACCES')) {
          errorType = 'permission_denied';
        } else if (error.message.includes('NETWORK')) {
          errorType = 'network_error';
        } else if (error.message.includes('npm')) {
          errorType = 'npm_error';
        }
      }

      trackCommandError('create', error, duration, {
        init: options.init,
        command: createCommand,
        errorType,
        exitCode: error.status || error.code,
      });

      console.error('Failed to generate Tonk code:', error);
      shutdownAnalytics();
      process.exit(1);
    }
  });

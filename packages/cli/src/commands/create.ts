import {Command} from 'commander';
import child_process from 'child_process';

export const createCommand = new Command('create')
  .description('Create a new tonk application or component')
  .argument('[type]', 'Type of template to scaffold')
  .action(typeArg => {
    try {
      // Build the command with any passed options
      let createCommand = `npx @tonk/create ${typeArg || 'app'}`;

      console.log(createCommand);

      // Execute the create package
      child_process.execSync(createCommand, {
        stdio: 'inherit',
        env: {...process.env},
      });

      process.exit(0);
    } catch (error: any) {
      if (error.status == 8) {
      } else {
        console.error('Failed to generate Tonk code:', error);
      }
      process.exit(1);
    }
  });

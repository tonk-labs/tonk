import {Command} from 'commander';
import child_process from 'child_process';

export const createCommand = new Command('create')
  .description('Create a new tonk application or component')
  .argument('[name]', 'Name of the application')
  .action(name => {
    console.log('Creating a new tonk app...');

    try {
      // Build the command with any passed options
      let createCommand = 'npx @tonk/create';

      // Pass through any options provided to the CLI
      if (name) createCommand += ` "${name}"`;

      console.log(createCommand);

      // Execute the create package
      child_process.execSync(createCommand, {
        stdio: 'inherit',
        env: {...process.env},
      });

      console.log('🎉 Tonk app created successfully!');
      process.exit(0);
    } catch (error) {
      console.error('Failed to create Tonk app:', error);
      process.exit(1);
    }
  });

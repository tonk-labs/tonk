import {Command} from 'commander';
import {
  registerInspectCommand,
  registerListCommand,
  registerRemoveCommand,
  registerPingCommand,
  registerStartCommand,
  registerStopCommand,
  registerLogsCommand,
  registerRegisterCommand,
  registerInstallCommand,
  registerInitCommand,
} from './commands.js';

/**
 * Create and configure the worker command
 */
export function createWorkerCommand(): Command {
  const workerCommand = new Command('worker')
    .description('Manage Tonk workers')
    .action(async () => {
      // Display help when no subcommand is provided
      workerCommand.help();
    });

  // Register all subcommands
  registerInspectCommand(workerCommand);
  registerListCommand(workerCommand);
  registerRemoveCommand(workerCommand);
  registerPingCommand(workerCommand);
  registerStartCommand(workerCommand);
  registerStopCommand(workerCommand);
  registerLogsCommand(workerCommand);
  registerRegisterCommand(workerCommand);
  registerInstallCommand(workerCommand);
  registerInitCommand(workerCommand);

  return workerCommand;
}

// Export the worker command
export const workerCommand = createWorkerCommand();

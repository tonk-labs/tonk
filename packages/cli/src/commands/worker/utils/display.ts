import chalk from 'chalk';
import {Worker} from '../../../types/worker.js';

/**
 * Display detailed information about a worker
 */
export function displayWorkerDetails(worker: Worker): void {
  console.log(chalk.bold(`Worker Details: ${worker.name}`));
  console.log(chalk.cyan('ID:'), worker.id);
  console.log(chalk.cyan('Name:'), worker.name);
  console.log(
    chalk.cyan('Description:'),
    worker.description || '(No description)',
  );
  console.log(chalk.cyan('Endpoint:'), worker.endpoint);
  console.log(chalk.cyan('Protocol:'), worker.protocol);
  console.log(
    chalk.cyan('Status:'),
    worker.status.active ? chalk.green('Active') : chalk.yellow('Inactive'),
  );

  if (worker.status.lastSeen) {
    console.log(
      chalk.cyan('Last Seen:'),
      new Date(worker.status.lastSeen).toLocaleString(),
    );
  } else {
    console.log(chalk.cyan('Last Seen:'), 'Never');
  }

  console.log(
    chalk.cyan('Created:'),
    new Date(worker.createdAt).toLocaleString(),
  );
  console.log(
    chalk.cyan('Updated:'),
    new Date(worker.updatedAt).toLocaleString(),
  );

  if (Object.keys(worker.env).length > 0) {
    console.log(chalk.cyan('\nEnvironment Variables:'));
    Object.entries(worker.env).forEach(([key, value]) => {
      console.log(`  ${key}=${value}`);
    });
  }

  if (Object.keys(worker.config).length > 0) {
    console.log(chalk.cyan('\nConfiguration:'));
    console.log(JSON.stringify(worker.config, null, 2));
  }
}

#!/usr/bin/env node
import {Command} from 'commander';
import chalk from 'chalk';
import {spawn} from 'child_process';
import path from 'path';
import {fileURLToPath} from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const program = new Command();

interface ServerError {
  code?: number;
  message: string;
}

// Add start-mcp command
program
  .command('start')
  .description('Start the MCP server')
  .action(async () => {
    console.log(chalk.cyan('Starting MCP server...'));

    // Spawn the MCP server process using the installed package
    const serverProcess = spawn('tinyfoot-mcp-server', ['start'], {
      stdio: 'inherit',
      shell: true,
      env: {...process.env},
    });

    // Handle process events
    serverProcess.on('error', error => {
      console.error(chalk.red('Failed to start server:'), error.message);
      throw error;
    });

    // Handle clean exit
    const handleShutdown = () => {
      console.log(chalk.yellow('\nShutting down MCP server...'));
      serverProcess.kill();
      // Only exit after server process is fully terminated
      serverProcess.on('close', () => {
        process.exit(0);
      });
    };

    process.on('SIGINT', handleShutdown);
    process.on('SIGTERM', handleShutdown);
  });

// Main program setup
program
  .name('tinyfoot')
  .description('Tinyfoot CLI - MCP Server')
  .version('0.1.0')
  .on('--help', () => {
    console.log('\nAvailable Commands:');
    console.log('  start       Start the MCP server');
    console.log('\nExamples:');
    console.log('  $ tinyfoot start');
  });

// Parse arguments
program.parse(process.argv);

// Show help if no arguments provided
if (process.argv.length <= 2) {
  program.help();
}

#!/usr/bin/env node
import { Command } from 'commander';
import chalk from 'chalk';
import ora from 'ora';
import fs from 'fs-extra';
import path from 'path';
import os from 'os';
import { fileURLToPath } from 'url';
import readline from 'readline';
import { spawn } from 'child_process';
import taskPrompt from './taskPrompt';

import OllamaClient from './ollamaClient';

// Create __dirname equivalent for ES modules
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const program = new Command();
const ollamaClient = new OllamaClient();

// Helper function to check if running in a Tinyfoot project
function isTinyfootProject() {
  try {
    return fs.existsSync('tinyfoot.config.json');
  } catch (error) {
    return false;
  }
}

// Helper function to read tinyfoot.config.json
async function readConfig() {
  try {
    if (isTinyfootProject()) {
      return await fs.readJSON('tinyfoot.config.json');
    }
    return null;
  } catch (error) {
    console.error(chalk.red('Error reading tinyfoot.config.json:'), error);
    return null;
  }
}

// Function to query Ollama and get task JSON
async function queryOllama(prompt: string) {
  // Use the OllamaClient to send the chat request
  return ollamaClient.chat(prompt);
}

// Function to get the path to implement.log in user's home directory
function getImplementLogPath() {
  // Get user's home directory
  const homeDir = os.homedir();
  // Create .tinyfoot directory path
  const tinyfootDir = path.join(homeDir, '.tinyfoot');
  
  // Create the .tinyfoot directory if it doesn't exist
  if (!fs.existsSync(tinyfootDir)) {
    fs.mkdirSync(tinyfootDir, { recursive: true });
  }
  
  // Return the full path to the implement.log file
  return path.join(tinyfootDir, 'implement.log');
}

// Function to append task to implement.log
async function appendToImplementLog(task: any) {
  try {
    const logPath = getImplementLogPath();
    
    // Create implement.log if it doesn't exist
    if (!fs.existsSync(logPath)) {
      await fs.writeFile(logPath, '');
    }
    
    // Format the task as a nice JSON string
    const taskEntry = JSON.stringify(task, null, 2);
    
    // Append to the log file with a timestamp and separator
    await fs.appendFile(
      logPath, 
      `\n--- ${new Date().toISOString()} ---\n${taskEntry}\n\n`
    );
    
    console.log(chalk.green(`Task appended to ${logPath}`));
  } catch (error) {
    console.error(chalk.red('Error appending to implement.log:'), error);
  }
}

// Add chat command
program
  .command('chat')
  .description('Start an interactive chat with Ollama')
  .option('-m, --model <model>', 'Specify the Ollama model to use (defaults to deepseek-r1:32b)')
  .action(async (options) => {
    // If a model was specified, use it
    if (options.model) {
      ollamaClient.setModel(options.model);
    }
    
    console.log(chalk.cyan(`Starting chat with Ollama using model: ${ollamaClient.getModel()}`));
    console.log(chalk.yellow('Type "exit", "quit", or press Ctrl+C to end the chat'));
    
    // Create the readline interface
    const rl = readline.createInterface({
      input: process.stdin,
      output: process.stdout
    });
    
    // Store the conversation history
    const messages: Array<{ role: string; content: string }> = [];
    
    messages.push({ role: 'system', content: taskPrompt });
    
    // Function to clean up and exit
    const cleanupAndExit = () => {
      rl.close();
      setTimeout(() => {
        process.exit(0);
      }, 100);
    };
    
    // Handle SIGINT (Ctrl+C)
    process.on('SIGINT', () => {
      console.log(chalk.green('\nChat ended with Ctrl+C. Goodbye!'));
      cleanupAndExit();
    });
    
    // Use a simple prompt for input
    const displayPrompt = () => {
      process.stdout.write(chalk.blue('You: '));
    };
    
    
    rl.on('line', async (line) => {
      // Get the input
      const userInput = line.trim();
      
      // Check for exit command
      if (userInput.toLowerCase() === 'exit' || userInput.toLowerCase() === 'quit') {
        console.log(chalk.green('Chat ended. Goodbye!'));
        cleanupAndExit();
        return;
      }
      
      // Add user message to history
      messages.push({ role: 'user', content: userInput });
      
      // Pause readline to prevent input during processing
      rl.pause();
      
      // Show spinner while waiting for response
      const spinner = ora('Waiting for response...').start();
      let isFirstChunk = true;
      
      try {
        await ollamaClient.chatStream(
          messages,
          // Chunk handler
          (chunk) => {
            if (isFirstChunk) {
              spinner.stop();
              process.stdout.write(chalk.green('AI: '));
              isFirstChunk = false;
            }
            process.stdout.write(chunk);
          },
          // Complete handler
          (fullResponse) => {
            // Add assistant response to history
            messages.push({ role: 'assistant', content: fullResponse });
            console.log('\n');
            
            // Resume readline and display prompt again
            rl.resume();
            displayPrompt();
          }
        );
      } catch (error) {
        spinner.fail('Error getting response');
        console.error(chalk.red('Error:'), error);
        
        // Resume readline even on error
        rl.resume();
        displayPrompt();
      }
    });
    
    // Initial prompt
    displayPrompt();
  });



// Add ask command
// program
//   .command('ask')
//   .description('Ask Ollama to create a task and append it to implement.log')
//   .argument('<question>', 'The question or task description to send to Ollama')
//   .option('-m, --model <model>', 'Specify the Ollama model to use (defaults to deepseek-r1:32b)')
//   .action(async (question: string, options) => {
//     // Check if we're in a Tinyfoot project
//   });

// Add view-log command
program
  .command('view-log')
  .description('View the implementation log')
  .option('-n, --lines <number>', 'Number of recent entries to show', '10')
  .action(async (options) => {
    try {
      if (!fs.existsSync('implement.log')) {
        console.log(chalk.yellow('implement.log does not exist yet. Use "tinyfoot ask" to create tasks.'));
        return;
      }
      
      const logContent = await fs.readFile('implement.log', 'utf-8');
      const entries = logContent.split('\n--- ').filter(Boolean);
      
      const numEntries = Math.min(parseInt(options.lines), entries.length);
      const recentEntries = entries.slice(-numEntries);
      
      console.log(chalk.cyan(`Showing ${numEntries} recent entries from implement.log:`));
      recentEntries.forEach(entry => {
        console.log(chalk.yellow('\n--- ') + entry);
      });
    } catch (error) {
      console.error(chalk.red('Error reading implement.log:'), error);
    }
  });

// Add info command to display project information
program
  .command('info')
  .description('Display information about the current Tinyfoot project')
  .action(async () => {
    const config = await readConfig();
    
    if (!config) {
      console.log(chalk.yellow('Not in a Tinyfoot project directory. Run this command from a Tinyfoot project root.'));
      return;
    }
    
    console.log(chalk.cyan('Tinyfoot Project Information:'));
    console.log(chalk.bold('Name:'), config.name);
    console.log(chalk.bold('Description:'), config.plan.projectDescription);
    
    if (config.plan.implementationLog && config.plan.implementationLog.length > 0) {
      console.log(chalk.bold('\nImplementation Log:'));
      config.plan.implementationLog.forEach((entry: string, index: number) => {
        console.log(`${index + 1}. ${entry}`);
      });
    }
  });

// Add start-mcp command
program
  .command('start')
  .description('Start the MCP server in the mcp folder')
  .option('-p, --port <port>', 'Specify the port to run the server on (defaults to 3000)')
  .option('-h, --host <host>', 'Specify the host to bind to (defaults to localhost)')
  .action(async (options) => {
    const port = parseInt(options.port || '4321', 10);
    const host = options.host || 'localhost';
    
    console.log(chalk.cyan(`Starting MCP server on ${host}:${port}...`));
    
    try {
      const __filename = fileURLToPath(import.meta.url);
      const __dirname = path.dirname(__filename);
      
      // Spawn the child process
      const serverProcess = spawn('node', [
        path.join(__dirname, '..', 'lib', 'tinyfoot-mcp', 'dist', 'index.js'),
        '--port', port.toString(),
        '--host', host
      ]);
      
      // Forward the output from the child process to the parent process
      serverProcess.stdout.on('data', (data) => {
        console.log(data.toString());
      });
      
      serverProcess.stderr.on('data', (data) => {
        console.error(chalk.red(data.toString()));
      });
      
      // Handle child process exit
      serverProcess.on('exit', (code) => {
        if (code !== 0) {
          console.log(chalk.red(`MCP server exited with code ${code}`));
        }
        process.exit(code || 0);
      });
      
      // Ensure child process is killed when parent process exits
      process.on('exit', () => {
        serverProcess.kill();
      });
      
      // Handle SIGINT and SIGTERM to gracefully exit
      process.on('SIGINT', () => {
        console.log(chalk.yellow('\nShutting down MCP server...'));
        serverProcess.kill();
        process.exit(0);
      });
      
      process.on('SIGTERM', () => {
        console.log(chalk.yellow('\nShutting down MCP server...'));
        serverProcess.kill();
        process.exit(0);
      });
      
    } catch (error) {
      console.error(chalk.red('Error starting MCP server:'), error);
      process.exit(1);
    }
  });

// Main program setup
program
  .name('tinyfoot')
  .description('Tinyfoot CLI - Helpful commands for Tinyfoot projects')
  .version('0.1.0')
  .on('--help', () => {
    console.log('\nAvailable Commands:');
    console.log('  ask         Ask Ollama to create a task and append it to implement.log');
    console.log('  chat        Start an interactive chat with Ollama');
    console.log('  view-log    View recent entries in the implementation log');
    console.log('  info        Display information about the current Tinyfoot project');
    console.log('  start   Start the MCP server in the mcp folder');
    console.log('\nExamples:');
    console.log('  $ tinyfoot ask "Create a new component for user profile"');
    console.log('  $ tinyfoot ask -m llama3 "Create a login form"');
    console.log('  $ tinyfoot chat');
    console.log('  $ tinyfoot chat -m llama3');
    console.log('  $ tinyfoot view-log --lines 5');
    console.log('  $ tinyfoot info');
    console.log('  $ tinyfoot start');
    console.log('  $ tinyfoot start -p 3001 -h 0.0.0.0');
  });

// Parse arguments
program.parse(process.argv);

// Show help if no arguments provided
if (process.argv.length <= 2) {
  program.help();
} 
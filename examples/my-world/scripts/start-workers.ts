#!/usr/bin/env node

import { execSync } from "child_process";
import * as fs from "fs";
import * as path from "path";

// Worker management interfaces
interface TonkConfig {
  name: string;
  workers?: string[];
  [key: string]: any;
}

/**
 * Get workers from tonk.config.json
 */
function getWorkers(): string[] {
  try {
    // Read tonk.config.json to get worker dependencies
    // When running from scripts directory, we need to go up one level
    const configPath = path.resolve(
      process.cwd().includes('scripts') ? path.resolve(process.cwd(), '..') : process.cwd(), 
      "tonk.config.json"
    );

    if (!fs.existsSync(configPath)) {
      console.warn("No tonk.config.json found.");
      return [];
    }

    const config: TonkConfig = JSON.parse(fs.readFileSync(configPath, "utf8"));
    return config.workers || [];
  } catch (error) {
    console.error("Error reading tonk.config.json:", error);
    return [];
  }
}

/**
 * Install and start workers
 */
async function startWorkers(): Promise<void> {
  console.log('Setting up workers for my-world application...');
  
  try {
    const workers = getWorkers();

    if (workers.length === 0) {
      console.log("No worker dependencies specified in tonk.config.json");
      return;
    }

    console.log(`Setting up ${workers.length} worker dependencies...`);

    // Install each worker package if not already installed
    // Note: 'tonk worker install' already starts the worker, so we don't need to start it again
    for (const worker of workers) {
      console.log(`Installing and starting worker: ${worker}`);
      try {
        execSync(`tonk worker install ${worker}`, { stdio: "inherit" });
      } catch (error) {
        console.error(`Error installing worker ${worker}:`, error);
        // Continue with other workers even if one fails
      }
    }

    try {
      console.log("Worker setup complete!");
      
      // Display worker status
      console.log("Checking worker status...");
      execSync("tonk worker status", { stdio: "inherit" });
      
      console.log('\nWorkers are now running. To stop workers, use:');
      console.log('  npm run workers:stop');
    } catch (error) {
      console.error("Error checking worker status:", error);
      process.exit(1);
    }
  } catch (error) {
    console.error("Error setting up workers:", error);
    process.exit(1);
  }
}

/**
 * Stop all workers
 */
async function stopWorkers(): Promise<void> {
  console.log('Stopping workers...');
  
  try {
    const workers = getWorkers();
    
    if (workers.length === 0) {
      console.log("No worker dependencies specified in tonk.config.json");
      return;
    }
    
    for (const worker of workers) {
      console.log(`Stopping worker: ${worker}`);
      try {
        execSync(`tonk worker stop ${worker}`, { stdio: "inherit" });
      } catch (error) {
        console.error(`Error stopping worker ${worker}:`, error);
      }
    }
    
    console.log("All workers stopped.");
  } catch (error) {
    console.error("Error stopping workers:", error);
    process.exit(1);
  }
}

/**
 * Check status of all workers
 */
async function checkWorkersStatus(): Promise<void> {
  console.log('Checking worker status...');
  
  try {
    execSync("tonk worker status", { stdio: "inherit" });
  } catch (error) {
    console.error("Error checking worker status:", error);
    process.exit(1);
  }
}

/**
 * Main function to handle worker operations
 */
async function main(): Promise<void> {
  const command = process.argv[2] || 'start';
  
  switch (command) {
    case 'start':
      await startWorkers();
      break;
    case 'stop':
      await stopWorkers();
      break;
    case 'status':
      await checkWorkersStatus();
      break;
    default:
      console.log('Unknown command. Available commands: start, stop, status');
      process.exit(1);
  }
}

main();

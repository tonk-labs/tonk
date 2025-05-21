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
 * Install and start required workers for this application
 */
export async function setupWorkers(): Promise<void> {
  try {
    // Read tonk.config.json to get worker dependencies
    const configPath = path.resolve(process.cwd(), 'tonk.config.json');
    
    if (!fs.existsSync(configPath)) {
      console.warn('No tonk.config.json found. Skipping worker setup.');
      return;
    }
    
    const config: TonkConfig = JSON.parse(fs.readFileSync(configPath, 'utf8'));
    const workers = config.workers || [];
    
    if (workers.length === 0) {
      console.log('No worker dependencies specified in tonk.config.json');
      return;
    }
    
    console.log(`Installing ${workers.length} worker dependencies...`);
    
    // Install each worker package
    for (const worker of workers) {
      console.log(`Installing worker: ${worker}`);
      execSync(`tonk worker install ${worker}`, { stdio: 'inherit' });
    }
    
    // Start all workers
    console.log('Starting workers...');
    execSync('tonk worker start', { stdio: 'inherit' });
    
    console.log('Worker setup complete!');
  } catch (error) {
    console.error('Error setting up workers:', error);
    throw error;
  }
}

/**
 * Stop all running workers
 */
export function stopWorkers(): void {
  try {
    console.log('Stopping workers...');
    execSync('tonk worker stop', { stdio: 'inherit' });
    console.log('Workers stopped.');
  } catch (error) {
    console.error('Error stopping workers:', error);
    throw error;
  }
}

/**
 * Check status of all workers
 */
export function checkWorkersStatus(): void {
  try {
    console.log('Checking worker status...');
    execSync('tonk worker status', { stdio: 'inherit' });
  } catch (error) {
    console.error('Error checking worker status:', error);
    throw error;
  }
}

import fs from 'node:fs';
import path from 'node:path';
import envPaths from 'env-paths';
import {
  Worker,
  WorkerManager,
  WorkerRegistrationOptions,
} from '../types/worker.js';
import {WorkerConfigSchema} from '../types/workerConfig.js';
import http from 'node:http';
import https from 'node:https';
import {exec} from 'node:child_process';
import {promisify} from 'node:util';
import * as YAML from 'yaml';

/**
 * Implementation of the WorkerManager interface
 */
export class TonkWorkerManager implements WorkerManager {
  private workersPath: string;

  constructor() {
    // Create paths for Tonk daemon home
    const paths = envPaths('tonk', {suffix: ''});
    const tonkHome = paths.data;
    this.workersPath = path.join(tonkHome, 'workers');

    // Ensure workers directory exists
    if (!fs.existsSync(this.workersPath)) {
      fs.mkdirSync(this.workersPath, {recursive: true});
    }
  }

  /**
   * Register a new worker
   */
  async register(options: WorkerRegistrationOptions): Promise<Worker> {
    // Generate a unique ID for the worker
    const workerId = this.generateWorkerId(options.name);

    // Parse environment variables
    const env: Record<string, string> = {};
    if (options.env && Array.isArray(options.env)) {
      options.env.forEach((envVar: string) => {
        const [key, value] = envVar.split('=');
        if (key && value) {
          env[key] = value;
        }
      });
    }

    let workerConfig = {
      type: options.type || 'custom',
    };

    // If config file provided, load it
    if (options.config && fs.existsSync(options.config)) {
      try {
        const configContent = fs.readFileSync(options.config, 'utf-8');

        // Check if it's a YAML file
        if (
          options.config.endsWith('.yml') ||
          options.config.endsWith('.yaml')
        ) {
          try {
            const config = YAML.parse(configContent);

            // Validate against schema
            const result = WorkerConfigSchema.safeParse(config);
            if (result.success) {
              // Use the YAML config to populate worker fields
              return this.registerFromYamlConfig(result.data);
            } else {
              console.error('Invalid YAML configuration:', result.error);
              // Continue with basic registration
            }
          } catch (yamlError) {
            console.error('Error parsing YAML config:', yamlError);
          }
        } else {
          // Assume JSON
          const config = JSON.parse(configContent);
          workerConfig = {...workerConfig, ...config};
        }
      } catch (error) {
        console.error('Error loading config file:', error);
      }
    }

    // Create worker object
    const worker: Worker = {
      id: workerId,
      name: options.name,
      description: options.description || '',
      endpoint: options.endpoint,
      protocol: options.protocol || 'http',
      status: {
        active: false,
        lastSeen: null,
      },
      env,
      config: workerConfig,
      createdAt: Date.now(),
      updatedAt: Date.now(),
    };

    // Save worker to file
    const workerFilePath = path.join(this.workersPath, `${workerId}.json`);
    fs.writeFileSync(workerFilePath, JSON.stringify(worker, null, 2));

    return worker;
  }

  /**
   * Register a worker from a YAML configuration file
   */
  async registerFromYamlConfig(yamlConfig: any): Promise<Worker> {
    // Generate a unique ID for the worker
    const workerId = this.generateWorkerId(yamlConfig.name);
    // Create worker object from YAML config
    const worker: Worker = {
      id: workerId,
      name: yamlConfig.name,
      description: yamlConfig.description || '',
      endpoint: yamlConfig.endpoint,
      protocol: yamlConfig.protocol || 'http',
      status: {
        active: false,
        lastSeen: null,
      },
      env: yamlConfig.process?.env || {},
      config: {
        type: yamlConfig.type || 'custom',
        version: yamlConfig.version,
        healthCheck: yamlConfig.healthCheck,
        process: yamlConfig.process,
        ...yamlConfig.config,
      },
      createdAt: Date.now(),
      updatedAt: Date.now(),
    };

    // Save worker to file
    const workerFilePath = path.join(this.workersPath, `${workerId}.json`);
    fs.writeFileSync(workerFilePath, JSON.stringify(worker, null, 2));

    return worker;
  }

  /**
   * Get a worker by ID
   */
  async get(id: string): Promise<Worker | null> {
    const workerFilePath = path.join(this.workersPath, `${id}.json`);

    if (!fs.existsSync(workerFilePath)) {
      return null;
    }

    const fileContent = fs.readFileSync(workerFilePath, 'utf-8');
    return JSON.parse(fileContent);
  }

  /**
   * List all registered workers
   */
  async list(): Promise<Worker[]> {
    const workerFiles = fs
      .readdirSync(this.workersPath)
      .filter(file => file.endsWith('.json'));

    if (workerFiles.length === 0) {
      return [];
    }

    return workerFiles.map(file => {
      const filePath = path.join(this.workersPath, file);
      const fileContent = fs.readFileSync(filePath, 'utf-8');
      return JSON.parse(fileContent);
    });
  }

  /**
   * Update a worker
   */
  async update(id: string, updates: Partial<Worker>): Promise<Worker> {
    const worker = await this.get(id);

    if (!worker) {
      throw new Error(`Worker with ID '${id}' not found.`);
    }

    // Update worker properties
    const updatedWorker: Worker = {
      ...worker,
      ...updates,
      updatedAt: Date.now(),
    };

    // Save updated worker
    const workerFilePath = path.join(this.workersPath, `${id}.json`);
    fs.writeFileSync(workerFilePath, JSON.stringify(updatedWorker, null, 2));

    return updatedWorker;
  }

  /**
   * Remove a worker
   */
  async remove(id: string): Promise<boolean> {
    const workerFilePath = path.join(this.workersPath, `${id}.json`);

    if (!fs.existsSync(workerFilePath)) {
      return false;
    }

    // Get worker details before removing
    const worker = await this.get(id);
    if (!worker) {
      return false;
    }

    // Try to stop and delete from PM2 if it's running
    const execAsync = promisify(exec);
    try {
      // Check if worker is running in PM2
      const {stdout} = await execAsync('pm2 list');

      if (stdout.includes(`${worker.id}`)) {
        // Stop the worker with PM2 first
        await execAsync(`pm2 stop ${worker.id}`);
        // Then delete it from PM2
        await execAsync(`pm2 delete ${worker.id}`);
        console.log(`Worker '${worker.name}' stopped and removed from PM2.`);
      }
    } catch (error) {
      console.error(`Error stopping worker in PM2:`, error);
      // Continue with removal even if PM2 operations fail
    }

    // Remove worker file
    fs.unlinkSync(workerFilePath);
    return true;
  }

  /**
   * Start a worker
   * @param id Worker ID to start
   */
  async start(id: string): Promise<boolean> {
    const worker = await this.get(id);

    if (!worker) {
      throw new Error(`Worker with ID '${id}' not found.`);
    }

    const execAsync = promisify(exec);

    try {
      // Check if worker is already running in PM2
      const {stdout} = await execAsync('pm2 list');

      if (stdout.includes(`${worker.id}`)) {
        console.log(
          `Worker '${worker.name}' is already running. Restarting...`,
        );
        await execAsync(`pm2 restart ${worker.id}`);
      } else {
        // Prepare environment variables
        const envVars = Object.entries(worker.env)
          .map(([key, value]) => `${key}=${value}`)
          .join(' ');
        
        // Ensure log directory exists
        const logDir = envPaths('tonk', {suffix: ''}).log;
        if (!fs.existsSync(logDir)) {
          fs.mkdirSync(logDir, { recursive: true });
        }
        
        // Add log configuration
        const logConfig = `--log ${path.join(logDir, `${worker.id}.log`)}`;
        const errorLogConfig = `--error ${path.join(logDir, `${worker.id}-error.log`)}`;

        // Handle different worker types
        if (worker.config.type === 'npm') {
          // For npm-based workers, we need to find the entry point
          const packageName = worker.name;
          
          try {
            // Get the package's bin entry point
            const { stdout: binInfo } = await execAsync(`npm bin -g`);
            const binPath = binInfo.trim();
            const executablePath = path.join(binPath, packageName);
            
            // Check if the executable exists
            if (fs.existsSync(executablePath)) {
              // Start the npm package with PM2
              const startCmd = `${envVars} pm2 start ${executablePath} --name ${worker.id} ${logConfig} ${errorLogConfig}`;
              await execAsync(startCmd);
            } else {
              // Try to find the main entry point
              const { stdout: packageInfo } = await execAsync(`npm list -g ${packageName} --json`);
              const packageData = JSON.parse(packageInfo);
              
              if (packageData.dependencies && packageData.dependencies[packageName]) {
                const packagePath = packageData.dependencies[packageName].path;
                const packageJsonPath = path.join(packagePath, 'package.json');
                
                if (fs.existsSync(packageJsonPath)) {
                  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf-8'));
                  const mainScript = packageJson.main || 'index.js';
                  const scriptPath = path.join(packagePath, mainScript);
                  
                  // Start the npm package with PM2
                  const startCmd = `cd ${packagePath} && ${envVars} pm2 start ${scriptPath} --name ${worker.id} ${logConfig} ${errorLogConfig}`;
                  await execAsync(startCmd);
                } else {
                  throw new Error(`Could not find package.json for ${packageName}`);
                }
              } else {
                throw new Error(`Could not find installed package ${packageName}`);
              }
            }
          } catch (error) {
            console.error(`Error starting npm worker:`, error);
            throw error;
          }
        } else {
          // For traditional workers with process configuration
          if (!worker.config.process || !worker.config.process.script) {
            throw new Error(
              `Worker '${worker.name}' does not have a valid process configuration.`,
            );
          }
          
          // Start the worker with PM2
          const cwd = worker.config.process.cwd || '.';
          const instances = worker.config.process.instances || 1;
          const watch = worker.config.process.watch ? '--watch' : '';
          const maxMemory = worker.config.process.max_memory_restart
            ? `--max-memory-restart ${worker.config.process.max_memory_restart}`
            : '';

          const startCmd = `cd ${cwd} && ${envVars} pm2 start ${worker.config.process.script} --name ${worker.id} --instances ${instances} ${watch} ${maxMemory} ${logConfig} ${errorLogConfig}`;
          await execAsync(startCmd);
        }
      }

      // Update worker status
      await this.update(id, {
        status: {
          active: true,
          lastSeen: Date.now(),
        },
      });

      return true;
    } catch (error) {
      console.error(`Error starting worker '${worker.name}':`, error);
      
      // Log detailed error information
      if (error instanceof Error) {
        console.error(`Error message: ${error.message}`);
        console.error(`Stack trace: ${error.stack}`);
      }
      
      return false;
    }
  }

  /**
   * Stop a worker
   */
  async stop(id: string): Promise<boolean> {
    const worker = await this.get(id);

    if (!worker) {
      throw new Error(`Worker with ID '${id}' not found.`);
    }

    const execAsync = promisify(exec);

    try {
      console.log(`Stopping worker '${worker.name}'...`);

      // Check if worker is running in PM2
      const {stdout} = await execAsync('pm2 list');

      if (stdout.includes(`${worker.id}`)) {
        // Stop the worker with PM2
        await execAsync(`pm2 stop ${worker.id}`);
      } else {
        console.log(`Worker '${worker.name}' is not running.`);
      }

      // Update worker status
      await this.update(id, {
        status: {
          active: false,
          lastSeen: worker.status.lastSeen,
        },
      });

      return true;
    } catch (error) {
      console.error(`Error stopping worker:`, error);
      return false;
    }
  }

  /**
   * Check worker health
   */
  async checkHealth(id: string): Promise<boolean> {
    const worker = await this.get(id);

    if (!worker) {
      throw new Error(`Worker with ID '${id}' not found.`);
    }

    // If no health check configured, just ping the endpoint
    const healthCheckEndpoint =
      worker.config.healthCheck?.endpoint || worker.endpoint;
    const healthCheckTimeout = worker.config.healthCheck?.timeout || 5000;

    try {
      // Perform health check based on protocol
      if (worker.protocol === 'http' || worker.protocol === 'https') {
        const isHealthy = await this.httpHealthCheck(
          healthCheckEndpoint,
          worker.protocol,
          healthCheckTimeout,
        );

        // Update worker status
        await this.update(id, {
          status: {
            active: isHealthy,
            lastSeen: isHealthy ? Date.now() : worker.status.lastSeen,
          },
        });

        return isHealthy;
      }

      // For other protocols, we'll need to implement specific health checks
      // For now, just return false for unsupported protocols
      console.log(
        `Health check for protocol '${worker.protocol}' not implemented.`,
      );
      return false;
    } catch (error) {
      console.error(`Error checking worker health:`, error);

      // Update worker status to inactive
      await this.update(id, {
        status: {
          active: false,
          lastSeen: worker.status.lastSeen,
        },
      });

      return false;
    }
  }

  /**
   * Generate a worker ID
   */
  private generateWorkerId(name: string): string {
    const normalizedName = name.toLowerCase().replace(/[^a-z0-9]/g, '-');
    const timestamp = Date.now().toString(36);
    return `${normalizedName}-${timestamp}`;
  }

  /**
   * Perform HTTP health check
   */
  private httpHealthCheck(
    endpoint: string,
    protocol: string,
    timeout: number,
  ): Promise<boolean> {
    return new Promise(resolve => {
      const client = protocol === 'https' ? https : http;
      const req = client.get(endpoint, res => {
        // Consider 2xx status codes as healthy
        const isHealthy =
          res.statusCode !== undefined &&
          res.statusCode >= 200 &&
          res.statusCode < 300;
        resolve(isHealthy);
      });

      req.on('error', () => {
        resolve(false);
      });

      req.setTimeout(timeout, () => {
        req.destroy();
        resolve(false);
      });
    });
  }
}

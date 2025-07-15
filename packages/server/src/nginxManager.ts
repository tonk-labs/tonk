import fs from 'node:fs';
import path from 'node:path';
import {spawn} from 'node:child_process';
import chalk from 'chalk';
import {logger} from './logger.js';

import type {ChildProcess} from 'node:child_process';

interface CommandResult {
  code: number;
  stdout: string;
  stderr: string;
}

export class NginxManager {
  private nginxPort = 8080;
  private configDir = '/etc/nginx/proxies-enabled';
  private nginxProcess?: ChildProcess;
  private isRunning = false;

  constructor() {
    this.ensureConfigDirectory();
  }

  private log(color: 'green' | 'red' | 'blue' | 'yellow', message: string) {
    if (color === 'blue') {
      logger.debug(chalk[color](message));
    } else {
      console.log(chalk[color](message));
    }
  }

  /**
   * Start nginx server on dedicated port
   */
  async start(): Promise<void> {
    if (this.isRunning) {
      this.log('yellow', 'Nginx server is already running');
      return;
    }

    this.log('blue', `Starting nginx server on port ${this.nginxPort}`);

    try {
      // Create a basic nginx configuration for our dedicated server
      await this.createMainNginxConfig();

      // Start nginx server on dedicated port
      this.nginxProcess = spawn(
        'nginx',
        ['-g', 'daemon off;', '-c', this.getMainConfigPath()],
        {
          stdio: 'pipe',
        },
      );

      this.nginxProcess.stdout?.on('data', data => {
        this.log('blue', `[nginx] ${data.toString().trim()}`);
      });

      this.nginxProcess.stderr?.on('data', data => {
        this.log('red', `[nginx] ${data.toString().trim()}`);
      });

      this.nginxProcess.on('error', error => {
        this.log('red', `Nginx failed to start: ${error.message}`);
        this.isRunning = false;
      });

      this.nginxProcess.on('exit', (code, signal) => {
        this.log(
          'yellow',
          `Nginx process exited with code ${code}, signal ${signal}`,
        );
        this.isRunning = false;
      });

      this.isRunning = true;
      this.log('green', `✅ Nginx server started on port ${this.nginxPort}`);
    } catch (error) {
      this.log('red', `Failed to start nginx: ${error}`);
      throw error;
    }
  }

  /**
   * Stop nginx server
   */
  async stop(): Promise<void> {
    if (!this.isRunning || !this.nginxProcess) {
      this.log('yellow', 'Nginx server is not running');
      return;
    }

    this.log('blue', 'Stopping nginx server');

    // Gracefully terminate nginx
    this.nginxProcess.kill('SIGTERM');

    // Wait for graceful shutdown, then force kill if needed
    const killTimeout = setTimeout(() => {
      if (this.nginxProcess && !this.nginxProcess.killed) {
        this.log('yellow', 'Force killing nginx server');
        this.nginxProcess.kill('SIGKILL');
      }
    }, 5000);

    // Clean up when process exits
    this.nginxProcess.on('exit', () => {
      clearTimeout(killTimeout);
      this.isRunning = false;
      this.log('green', '✅ Nginx server stopped');
    });
  }

  /**
   * Deploy app-specific nginx configuration with processed content
   */
  async deployAppConfig(
    bundleName: string,
    processedConfigContent: string,
  ): Promise<void> {
    const targetConfigPath = path.join(
      this.configDir,
      `app-${bundleName}.conf`,
    );

    this.log(
      'blue',
      `Deploying nginx config for bundle "${bundleName}" with processed content`,
    );

    // Write the processed config content to nginx config directory
    fs.writeFileSync(targetConfigPath, processedConfigContent);

    // Validate and reload nginx
    await this.validateAndReload();

    this.log('green', `✅ Nginx config deployed for bundle "${bundleName}"`);
  }

  /**
   * Remove app-specific nginx configuration
   */
  async removeAppConfig(bundleName: string): Promise<void> {
    const configPath = path.join(this.configDir, `app-${bundleName}.conf`);

    if (fs.existsSync(configPath)) {
      this.log('blue', `Removing nginx config for bundle "${bundleName}"`);
      fs.unlinkSync(configPath);
      await this.validateAndReload();
      this.log('green', `✅ Nginx config removed for bundle "${bundleName}"`);
    }
  }

  /**
   * Get nginx server status
   */
  getStatus(): {isRunning: boolean; port: number; configDir: string} {
    return {
      isRunning: this.isRunning,
      port: this.nginxPort,
      configDir: this.configDir,
    };
  }

  /**
   * Validate nginx configuration and reload if valid
   */
  private async validateAndReload(): Promise<void> {
    if (!this.isRunning) {
      this.log('yellow', 'Nginx is not running, skipping reload');
      return;
    }

    this.log('blue', 'Validating nginx configuration');

    // Validate nginx configuration
    const validateResult = await this.runCommand('nginx', [
      '-t',
      '-c',
      this.getMainConfigPath(),
    ]);

    if (validateResult.code !== 0) {
      throw new Error(
        `Nginx config validation failed: ${validateResult.stderr}`,
      );
    }

    this.log('green', '✅ Nginx configuration is valid');

    // Graceful reload
    const reloadResult = await this.runCommand('nginx', [
      '-s',
      'reload',
      '-c',
      this.getMainConfigPath(),
    ]);

    if (reloadResult.code !== 0) {
      throw new Error(`Nginx reload failed: ${reloadResult.stderr}`);
    }

    this.log('green', '✅ Nginx configuration reloaded successfully');
  }

  /**
   * Ensure nginx config directory exists
   */
  private ensureConfigDirectory(): void {
    if (!fs.existsSync(this.configDir)) {
      fs.mkdirSync(this.configDir, {recursive: true});
      this.log('green', `Created nginx config directory: ${this.configDir}`);
    }
  }

  /**
   * Create main nginx configuration file
   */
  private async createMainNginxConfig(): Promise<void> {
    const mainConfigPath = this.getMainConfigPath();
    const mainConfig = `
events {
    worker_connections 1024;
}

http {
    include /etc/nginx/mime.types;
    default_type application/octet-stream;
    
    # Basic settings
    sendfile on;
    tcp_nopush on;
    tcp_nodelay on;
    keepalive_timeout 65;
    types_hash_max_size 2048;
    
    # Request body and upload settings
    client_max_body_size 100M;
    client_body_timeout 60s;
    client_header_timeout 60s;
    
    # Proxy settings for request preservation
    proxy_connect_timeout 60s;
    proxy_send_timeout 60s;
    proxy_read_timeout 60s;
    
    # Preserve original request headers
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header X-Forwarded-Host $host;
    proxy_set_header X-Forwarded-Port $server_port;
    
    # Preserve request body and headers
    proxy_pass_request_body on;
    proxy_pass_request_headers on;
    
    # Logging
    access_log /var/log/nginx/access.log;
    error_log /var/log/nginx/error.log;
    
    # Default server that handles all requests
    server {
        listen localhost:${this.nginxPort} default_server;
        server_name _;
        
        # Health check endpoint
        location /health {
            access_log off;
            return 200 '{"status":"healthy","service":"nginx-tonk","timestamp":"$time_iso8601"}';
            add_header Content-Type application/json;
        }
        
        # Include all app configurations (location blocks)
        include ${this.configDir}/*.conf;
        
        # All other requests return 404
        location / {
            return 404;
        }
    }
}
`;

    fs.writeFileSync(mainConfigPath, mainConfig);
  }

  /**
   * Get path to main nginx configuration file
   */
  private getMainConfigPath(): string {
    return '/etc/nginx/nginx.conf';
  }

  /**
   * Run a command and return the result
   */
  private runCommand(command: string, args: string[]): Promise<CommandResult> {
    return new Promise(resolve => {
      const process = spawn(command, args);
      let stdout = '';
      let stderr = '';

      process.stdout?.on('data', data => (stdout += data.toString()));
      process.stderr?.on('data', data => (stderr += data.toString()));

      process.on('close', code => {
        resolve({code: code || 0, stdout, stderr});
      });
    });
  }
}

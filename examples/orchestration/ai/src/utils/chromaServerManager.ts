import { spawn, ChildProcess } from "child_process";
import { writeFileSync, mkdirSync } from "fs";
import { join } from "path";

export class ChromaServerManager {
  private chromaProcess: ChildProcess | null = null;
  private port: number;
  private isStarting: boolean = false;

  constructor(port: number = 8888) {
    this.port = port;
  }

  private createChromaConfig(): string {
    const configPath = join(process.cwd(), "config.yaml");

    const config = `########################
# HTTP server settings #
########################
port: 8000
listen_address: "0.0.0.0"
cors_allow_origins: ["*"]

####################
# General settings #
####################
allow_reset: true
`;

    writeFileSync(configPath, config);
    return configPath;
  }

  async start(): Promise<void> {
    if (this.chromaProcess || this.isStarting) {
      console.log("Chroma server is already running or starting");
      return;
    }

    // Check if server is already running
    if (await this.checkHealth()) {
      console.log("✅ Chroma server is already running");
      return;
    }

    this.isStarting = true;

    return new Promise((resolve, reject) => {
      console.log(`Starting Chroma server on port ${this.port}...`);

      // Create config file with CORS enabled
      const configPath = this.createChromaConfig();
      const dataDir = join(process.cwd(), "chroma-data");

      try {
        mkdirSync(dataDir, { recursive: true });
      } catch (error) {
        // Directory might already exist
      }

      // Try to start Chroma via Docker first
      this.chromaProcess = spawn(
        "docker",
        [
          "run",
          "--rm",
          "-p",
          `${this.port}:8000`,
          "-v",
          `${dataDir}:/data`,
          "-v",
          `${configPath}:/config.yaml`,
          "chromadb/chroma",
        ],
        {
          stdio: ["ignore", "pipe", "pipe"],
        },
      );

      let hasStarted = false;
      let startupCheckInterval: NodeJS.Timeout;

      // Start polling for server readiness
      const checkServerReady = async () => {
        const isHealthy = await this.checkHealth();
        if (isHealthy && !hasStarted) {
          hasStarted = true;
          this.isStarting = false;
          if (startupCheckInterval) {
            clearInterval(startupCheckInterval);
          }
          console.log("✅ Chroma server started successfully");
          resolve();
        }
      };

      // Check every 2 seconds
      startupCheckInterval = setInterval(checkServerReady, 2000);

      // Handle stdout - just for logging
      this.chromaProcess.stdout?.on("data", (data) => {
        const output = data.toString();
        console.log(`Chroma: ${output.trim()}`);
      });

      // Handle stderr
      this.chromaProcess.stderr?.on("data", (data) => {
        const error = data.toString();
        console.error(`Chroma error: ${error.trim()}`);

        // If Docker is not available, try alternative methods
        if (
          error.includes("docker: command not found") ||
          error.includes("Cannot connect to the Docker daemon")
        ) {
          if (startupCheckInterval) {
            clearInterval(startupCheckInterval);
          }
          console.log(
            "Docker not available, attempting to start Chroma directly...",
          );
          this.startWithPython().then(resolve).catch(reject);
        }
      });

      // Handle process exit
      this.chromaProcess.on("exit", (code, signal) => {
        this.chromaProcess = null;
        this.isStarting = false;
        if (startupCheckInterval) {
          clearInterval(startupCheckInterval);
        }

        if (code !== 0 && !hasStarted) {
          console.log("Docker method failed, trying Python installation...");
          this.startWithPython().then(resolve).catch(reject);
        } else {
          console.log(
            `Chroma server exited with code ${code}, signal ${signal}`,
          );
        }
      });

      // Handle spawn error
      this.chromaProcess.on("error", (error) => {
        this.isStarting = false;
        if (startupCheckInterval) {
          clearInterval(startupCheckInterval);
        }
        console.error("Failed to start Chroma with Docker:", error.message);
        this.startWithPython().then(resolve).catch(reject);
      });

      // Timeout after 30 seconds
      setTimeout(() => {
        if (!hasStarted) {
          this.isStarting = false;
          if (startupCheckInterval) {
            clearInterval(startupCheckInterval);
          }
          reject(new Error("Chroma server startup timeout"));
        }
      }, 30000);
    });
  }

  private async startWithPython(): Promise<void> {
    return new Promise((resolve, reject) => {
      console.log("Attempting to start Chroma with Python...");

      // Try to start Chroma via pip installed version
      this.chromaProcess = spawn(
        "chroma",
        ["run", "--port", this.port.toString()],
        {
          stdio: ["ignore", "pipe", "pipe"],
        },
      );

      let hasStarted = false;
      let startupCheckInterval: NodeJS.Timeout;

      // Start polling for server readiness
      const checkServerReady = async () => {
        const isHealthy = await this.checkHealth();
        if (isHealthy && !hasStarted) {
          hasStarted = true;
          this.isStarting = false;
          if (startupCheckInterval) {
            clearInterval(startupCheckInterval);
          }
          console.log("✅ Chroma server started successfully with Python");
          resolve();
        }
      };

      // Check every 2 seconds
      startupCheckInterval = setInterval(checkServerReady, 2000);

      // Handle stdout - just for logging
      this.chromaProcess.stdout?.on("data", (data) => {
        const output = data.toString();
        console.log(`Chroma: ${output.trim()}`);
      });

      this.chromaProcess.stderr?.on("data", (data) => {
        const error = data.toString();
        console.error(`Chroma error: ${error.trim()}`);
      });

      this.chromaProcess.on("exit", (code) => {
        this.chromaProcess = null;
        this.isStarting = false;
        if (startupCheckInterval) {
          clearInterval(startupCheckInterval);
        }
        if (code !== 0 && !hasStarted) {
          reject(
            new Error(`Chroma server failed to start (exit code: ${code})`),
          );
        }
      });

      this.chromaProcess.on("error", (error) => {
        this.isStarting = false;
        if (startupCheckInterval) {
          clearInterval(startupCheckInterval);
        }
        reject(new Error(`Failed to start Chroma: ${error.message}`));
      });

      setTimeout(() => {
        if (!hasStarted) {
          this.isStarting = false;
          if (startupCheckInterval) {
            clearInterval(startupCheckInterval);
          }
          reject(new Error("Chroma server startup timeout (Python method)"));
        }
      }, 30000);
    });
  }

  async stop(): Promise<void> {
    return new Promise((resolve) => {
      if (!this.chromaProcess) {
        resolve();
        return;
      }

      console.log("Stopping Chroma server...");

      // Handle process exit
      this.chromaProcess.on("exit", () => {
        console.log("Chroma server stopped");
        this.chromaProcess = null;
        resolve();
      });

      // Try graceful shutdown first
      this.chromaProcess.kill("SIGTERM");

      // Force kill after 5 seconds
      setTimeout(() => {
        if (this.chromaProcess) {
          this.chromaProcess.kill("SIGKILL");
          this.chromaProcess = null;
          resolve();
        }
      }, 5000);
    });
  }

  isRunning(): boolean {
    return this.chromaProcess !== null && !this.chromaProcess.killed;
  }

  async checkHealth(): Promise<boolean> {
    try {
      const response = await fetch(
        `http://localhost:${this.port}/api/v2/heartbeat`,
      );
      return response.ok;
    } catch {
      return false;
    }
  }

  getPort(): number {
    return this.port;
  }
}

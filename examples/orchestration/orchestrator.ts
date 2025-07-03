#!/usr/bin/env node

import { readFileSync } from "fs";
import { resolve } from "path";
import {
  WorkflowExecutor,
  WorkflowExecutorConfig,
} from "./src/WorkflowExecutor";
import { Job } from "./src/JobRunner";
import { configureSyncEngine } from "@tonk/keepsync";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { NetworkAdapterInterface } from "@automerge/automerge-repo";
import { NodeFSStorageAdapter } from "@automerge/automerge-repo-storage-nodefs";

interface WorkflowFile {
  workflow: Job[];
}

interface OrchestratorConfig {
  workflowFile: string;
  workerUrl: string;
  verbose?: boolean;
  maxConcurrentJobs?: number;
  continueOnFailure?: boolean;
  timeout?: number;
}

class Orchestrator {
  private config: OrchestratorConfig;

  constructor(config: OrchestratorConfig) {
    this.config = config;
  }

  /**
   * Load workflow from file
   */
  private loadWorkflow(): Job[] {
    try {
      const workflowPath = resolve(this.config.workflowFile);

      if (this.config.verbose) {
        console.log(`📖 Loading workflow from: ${workflowPath}`);
      }

      // For TypeScript files, we need to require them
      if (workflowPath.endsWith(".ts") || workflowPath.endsWith(".js")) {
        delete require.cache[workflowPath];
        const workflowModule = require(workflowPath);

        // Handle both default export and named export
        const workflow =
          workflowModule.workflow ||
          workflowModule.default?.workflow ||
          workflowModule.default;

        if (!Array.isArray(workflow)) {
          throw new Error("Workflow file must export an array of jobs");
        }

        return workflow;
      }

      // For JSON files
      const content = readFileSync(workflowPath, "utf-8");
      const workflowFile: WorkflowFile = JSON.parse(content);

      if (!Array.isArray(workflowFile.workflow)) {
        throw new Error('Workflow file must contain a "workflow" array');
      }

      return workflowFile.workflow;
    } catch (error) {
      throw new Error(
        `Failed to load workflow: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  /**
   * Validate workflow structure
   */
  private validateWorkflow(jobs: Job[]): void {
    const jobIds = new Set<number>();

    for (const job of jobs) {
      // Check for duplicate job IDs
      if (jobIds.has(job.jobId)) {
        throw new Error(`Duplicate job ID found: ${job.jobId}`);
      }
      jobIds.add(job.jobId);

      // Validate job structure
      if (typeof job.jobId !== "number") {
        throw new Error(`Invalid job ID: ${job.jobId}. Must be a number.`);
      }

      if (!job.context || !job.context.template || !job.context.prompt) {
        throw new Error(
          `Job ${job.jobId} is missing required context (template and prompt)`,
        );
      }

      // Validate dependencies
      if (job.dependencies) {
        for (const depId of job.dependencies) {
          if (!jobIds.has(depId) && !jobs.some((j) => j.jobId === depId)) {
            throw new Error(
              `Job ${job.jobId} depends on non-existent job ${depId}`,
            );
          }
        }
      }
    }

    if (this.config.verbose) {
      console.log(`✅ Workflow validation passed: ${jobs.length} jobs`);
    }
  }

  /**
   * Execute the workflow
   */
  async execute(): Promise<void> {
    try {
      // Load and validate workflow
      const jobs = this.loadWorkflow();
      this.validateWorkflow(jobs);

      // Create executor configuration
      const executorConfig: WorkflowExecutorConfig = {
        workerUrl: this.config.workerUrl,
        verbose: this.config.verbose || false,
        maxConcurrentJobs: this.config.maxConcurrentJobs || 3,
        continueOnFailure: this.config.continueOnFailure || false,
        timeout: this.config.timeout || 300000, // 5 minutes default
      };

      // Execute workflow
      const executor = new WorkflowExecutor(executorConfig);
      const result = await executor.executeWorkflow(jobs);

      // Print results
      console.log("\\n" + "=".repeat(50));
      console.log("📊 WORKFLOW EXECUTION SUMMARY");
      console.log("=".repeat(50));

      console.log(`✅ Success: ${result.success}`);
      console.log(`📋 Total Jobs: ${result.summary.totalJobs}`);
      console.log(`✅ Successful: ${result.summary.successfulJobs}`);
      console.log(`❌ Failed: ${result.summary.failedJobs}`);

      if (result.summary.totalCost > 0) {
        console.log(`💰 Total Cost: $${result.summary.totalCost.toFixed(4)}`);
      }

      console.log("\\n📝 JOB RESULTS:");
      for (const jobResult of result.results) {
        const status = jobResult.success ? "✅" : "❌";
        console.log(
          `${status} Job ${jobResult.jobId}: ${jobResult.success ? "SUCCESS" : jobResult.error}`,
        );

        if (jobResult.success && jobResult.result && this.config.verbose) {
          console.log(
            `   Result: ${jobResult.result.substring(0, 100)}${jobResult.result.length > 100 ? "..." : ""}`,
          );
        }
      }

      // Exit with appropriate code
      process.exit(result.success ? 0 : 1);
    } catch (error) {
      console.error(
        "❌ Orchestration failed:",
        error instanceof Error ? error.message : String(error),
      );
      process.exit(1);
    }
  }
}

// CLI handling
function parseArgs(): OrchestratorConfig {
  const args = process.argv.slice(2);
  const config: Partial<OrchestratorConfig> = {};

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];

    switch (arg) {
      case "--workflow":
      case "-w":
        const workflowFile = args[++i];
        if (workflowFile) config.workflowFile = workflowFile;
        break;
      case "--worker-url":
      case "-u":
        const workerUrl = args[++i];
        if (workerUrl) config.workerUrl = workerUrl;
        break;
      case "--verbose":
      case "-v":
        config.verbose = true;
        break;
      case "--max-concurrent":
      case "-c":
        const maxConcurrent = args[++i];
        if (maxConcurrent) config.maxConcurrentJobs = parseInt(maxConcurrent);
        break;
      case "--continue-on-failure":
        config.continueOnFailure = true;
        break;
      case "--timeout":
      case "-t":
        const timeout = args[++i];
        if (timeout) config.timeout = parseInt(timeout) * 1000; // Convert seconds to milliseconds
        break;
      case "--help":
      case "-h":
        printHelp();
        process.exit(0);
      default:
        console.error(`Unknown argument: ${arg}`);
        printHelp();
        process.exit(1);
    }
  }

  // Set defaults
  if (!config.workflowFile) {
    config.workflowFile = "./workflow.ts";
  }

  if (!config.workerUrl) {
    config.workerUrl = process.env.WORKER_URL || "http://localhost:5556";
  }

  return config as OrchestratorConfig;
}

// keepsync
async function startKeepsync() {
  const SYNC_WS_URL = process.env.SYNC_WS_URL || "ws://localhost:7777/sync";
  const SYNC_URL = process.env.SYNC_URL || "http://localhost:7777";

  const wsAdapter = new BrowserWebSocketClientAdapter(SYNC_WS_URL);
  const engine = configureSyncEngine({
    url: SYNC_URL,
    network: [wsAdapter as any as NetworkAdapterInterface],
    storage: new NodeFSStorageAdapter(),
  });

  await engine.whenReady();
  console.log("✅ Keepsync engine is ready");
}

function printHelp(): void {
  console.log(`
🎯 Workflow Orchestrator

Usage: node orchestrator.ts [options]

Options:
  -w, --workflow <file>        Workflow file path (default: ./workflow.ts)
  -u, --worker-url <url>       AI worker URL (default: http://localhost:5556)
  -v, --verbose                Enable verbose logging
  -c, --max-concurrent <num>   Maximum concurrent jobs (default: 3)
  --continue-on-failure        Continue execution even if jobs fail
  -t, --timeout <seconds>      Request timeout in seconds (default: 300)
  -h, --help                   Show this help message

Environment Variables:
  WORKER_URL                   Default worker URL if not specified

Examples:
  node orchestrator.ts --workflow ./my-workflow.ts --verbose
  node orchestrator.ts -w workflow.json -u http://localhost:8080 -c 5
  `);
}

// Main execution
if (require.main === module) {
  const config = parseArgs();
  const orchestrator = new Orchestrator(config);
  await startKeepsync();
  orchestrator.execute().catch(console.error);
}

export { Orchestrator, OrchestratorConfig };


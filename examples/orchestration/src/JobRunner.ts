import { TemplateProcessor } from "./TemplateProcessor";
import { JobResult } from "./ContextManager";
import { LLMRequest, LLMResponse } from "../ai/src/services/claudeCodeProvider";

export interface Job {
  jobId: number;
  dependencies?: number[];
  context: {
    template: {
      path: string;
      source: any;
    };
    prompt: string;
    dependencyContext?: boolean;
  };
}

export interface JobRunnerConfig {
  workerUrl: string;
  verbose?: boolean;
  timeout?: number;
}

export class JobRunner {
  private templateProcessor: TemplateProcessor;
  private config: JobRunnerConfig;

  constructor(config: JobRunnerConfig) {
    this.config = config;
    this.templateProcessor = new TemplateProcessor();
  }

  /**
   * Execute a single job
   */
  async executeJob(
    job: Job,
    contextFromDependencies?: string,
  ): Promise<JobResult> {
    try {
      if (this.config.verbose) {
        console.log(`🚀 Starting job ${job.jobId}`);
      }

      // Create LLM request from template and prompt
      const contextToUse = job.context.dependencyContext ? contextFromDependencies : undefined;
      const llmRequest = await this.templateProcessor.createLLMRequest(
        job.context.template.path,
        job.context.prompt,
        contextToUse,
      );

      // Make request to AI worker
      const response = await this.makeWorkerRequest(llmRequest);

      if (response.success) {
        if (this.config.verbose) {
          console.log(`✅ Job ${job.jobId} completed successfully`);
          if (response.totalCostUsd) {
            console.log(`💰 Cost: $${response.totalCostUsd.toFixed(4)}`);
          }
        }

        return {
          jobId: job.jobId,
          success: true,
          result: response.content,
          metadata: {
            ...(response.totalCostUsd !== undefined && { totalCostUsd: response.totalCostUsd }),
            ...(response.sessionId !== undefined && { sessionId: response.sessionId }),
            ...(response.provider !== undefined && { provider: response.provider }),
          },
        };
      } else {
        if (this.config.verbose) {
          console.log(`❌ Job ${job.jobId} failed: ${response.error}`);
        }

        return {
          jobId: job.jobId,
          success: false,
          error: response.error || 'Unknown error',
          metadata: {
            ...(response.totalCostUsd !== undefined && { totalCostUsd: response.totalCostUsd }),
            ...(response.sessionId !== undefined && { sessionId: response.sessionId }),
            ...(response.provider !== undefined && { provider: response.provider }),
          },
        };
      }
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);

      if (this.config.verbose) {
        console.log(
          `💥 Job ${job.jobId} failed with exception: ${errorMessage}`,
        );
      }

      return {
        jobId: job.jobId,
        success: false,
        error: errorMessage,
      };
    }
  }

  /**
   * Make HTTP request to AI worker
   */
  private async makeWorkerRequest(
    llmRequest: LLMRequest,
  ): Promise<LLMResponse> {
    const controller = new AbortController();
    const timeoutId = setTimeout(
      () => controller.abort(),
      this.config.timeout || 300000,
    ); // 5 minute default timeout

    try {
      // Format the request to match what the AI worker expects
      const requestData = {
        prompt: llmRequest.prompt,
        systemPrompt: llmRequest.systemPrompt,
        maxTurns: llmRequest.maxTurns || 3,
        allowedTools: llmRequest.allowedTools,
        disallowedTools: llmRequest.disallowedTools,
        mcpConfig: llmRequest.mcpConfig,
        permissionMode: llmRequest.permissionMode || "default",
        verbose: this.config.verbose || false,
        outputFormat: "text",
      };

      const response = await fetch(`${this.config.workerUrl}/tonk`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify(requestData),
        signal: controller.signal,
      });

      clearTimeout(timeoutId);

      if (!response.ok) {
        throw new Error(`HTTP ${response.status}: ${response.statusText}`);
      }

      const result = await response.json();

      // The AI worker returns the LLMResponse directly
      return result as LLMResponse;
    } catch (error) {
      clearTimeout(timeoutId);

      if (error instanceof Error && error.name === "AbortError") {
        throw new Error("Request timed out");
      }

      throw error;
    }
  }

  /**
   * Check if the AI worker is healthy
   */
  async checkWorkerHealth(): Promise<boolean> {
    try {
      const response = await fetch(`${this.config.workerUrl}/health`, {
        method: "GET",
      });

      if (response.ok) {
        const health = await response.json();
        return health.status === "ok";
      }

      return false;
    } catch (error) {
      return false;
    }
  }
}


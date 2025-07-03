import { TemplateProcessor } from "./TemplateProcessor";
import { JobResult } from "./ContextManager";
import { LLMRequest, LLMResponse } from "../ai/src/services/claudeApiProvider";

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
  maxIterations?: number; // Maximum number of iterations for a job
  onUserInput?: (prompt: string) => Promise<string>; // Callback for user interaction
}

export class JobRunner {
  private templateProcessor: TemplateProcessor;
  private config: JobRunnerConfig;

  constructor(config: JobRunnerConfig) {
    this.config = config;
    this.templateProcessor = new TemplateProcessor();
  }

  /**
   * Execute a single job with support for iterations and user input
   */
  async executeJob(
    job: Job,
    contextFromDependencies?: string,
  ): Promise<JobResult> {
    const maxIterations = this.config.maxIterations || 1000;
    let iteration = 0;
    let conversationHistory: Array<{
      role: "user" | "assistant";
      content: string;
    }> = [];
    let totalCost = 0;
    let allContent: string[] = [];
    let lastSessionId: string | undefined;

    try {
      if (this.config.verbose) {
        console.log(`🚀 Starting job ${job.jobId}`);
        console.log(`📄 Template: ${job.context.template.path}`);
        console.log(`💬 Prompt: ${job.context.prompt}`);
      }

      // Create initial LLM request from template and prompt
      const contextToUse = job.context.dependencyContext
        ? contextFromDependencies
        : undefined;
      let llmRequest = await this.templateProcessor.createLLMRequest(
        job.context.template.path,
        job.context.prompt,
        contextToUse,
      );

      while (iteration < maxIterations) {
        iteration++;

        if (this.config.verbose) {
          console.log(
            `🔄 Job ${job.jobId} iteration ${iteration}/${maxIterations}`,
          );
        }

        // Add conversation history to request
        if (conversationHistory.length > 0) {
          llmRequest.conversationHistory = conversationHistory;
        }

        if (this.config.verbose) {
          console.log(
            `🔧 LLM Request:`,
            JSON.stringify(
              {
                ...llmRequest,
                conversationHistory: llmRequest.conversationHistory
                  ? `[${llmRequest.conversationHistory.length} messages]`
                  : undefined,
              },
              null,
              2,
            ),
          );
        }

        // Make request to AI worker
        const response = await this.makeWorkerRequest(llmRequest);

        if (!response.success) {
          if (this.config.verbose) {
            console.log(`❌ Job ${job.jobId} failed: ${response.error}`);
          }

          return {
            jobId: job.jobId,
            success: false,
            error: response.error || "Unknown error",
            metadata: {
              totalCostUsd: totalCost + (response.totalCostUsd || 0),
              ...(lastSessionId || response.sessionId
                ? { sessionId: lastSessionId || response.sessionId }
                : {}),
              ...(response.provider ? { provider: response.provider } : {}),
              iterations: iteration,
            },
          };
        }

        // Accumulate results
        allContent.push(response.content);
        totalCost += response.totalCostUsd || 0;
        lastSessionId = response.sessionId || lastSessionId;

        // Add to conversation history
        if (llmRequest.userInput) {
          conversationHistory.push({
            role: "user",
            content: llmRequest.userInput,
          });
        } else if (iteration === 1) {
          conversationHistory.push({
            role: "user",
            content: llmRequest.prompt,
          });
        }

        // Only add assistant response if it has content
        if (response.content && response.content.trim().length > 0) {
          conversationHistory.push({
            role: "assistant",
            content: response.content,
          });
        }

        // Check if job is finished
        if (response.isFinished) {
          if (this.config.verbose) {
            console.log(
              `✅ Job ${job.jobId} marked as finished by AI after ${iteration} iterations`,
            );
            if (totalCost > 0) {
              console.log(`💰 Total cost: $${totalCost.toFixed(4)}`);
            }
          }

          return {
            jobId: job.jobId,
            success: true,
            result: allContent.join("\n\n--- Iteration ---\n\n"),
            metadata: {
              totalCostUsd: totalCost,
              ...(lastSessionId ? { sessionId: lastSessionId } : {}),
              ...(response.provider ? { provider: response.provider } : {}),
              iterations: iteration,
            },
          };
        }

        // Check if user input is needed
        if (response.needsUserInput && response.userPrompt) {
          if (this.config.verbose) {
            console.log(
              `🤔 Job ${job.jobId} requesting user input: ${response.userPrompt}`,
            );
          }

          if (this.config.onUserInput) {
            const userResponse = await this.config.onUserInput(
              response.userPrompt,
            );

            if (this.config.verbose) {
              console.log(`👤 User responded: ${userResponse}`);
            }

            // Prepare next iteration with user input
            llmRequest = {
              ...llmRequest,
              userInput: userResponse,
              prompt: "Continue with the previous task.", // Provide a continuation prompt
            };
            continue;
          } else {
            // No user input handler configured
            return {
              jobId: job.jobId,
              success: false,
              error: `Job requires user input but no input handler configured. Prompt: ${response.userPrompt}`,
              metadata: {
                totalCostUsd: totalCost,
                ...(lastSessionId ? { sessionId: lastSessionId } : {}),
                ...(response.provider ? { provider: response.provider } : {}),
                iterations: iteration,
              },
            };
          }
        }

        // If not finished and no user input needed, continue with next iteration
        if (this.config.verbose) {
          console.log(`🔄 Job ${job.jobId} continuing to next iteration...`);
        }

        // Prepare for next iteration (AI wants to continue)
        llmRequest = {
          ...llmRequest,
          userInput: undefined,
          prompt: "Continue with the previous task.", // Provide a continuation prompt
        };
      }

      // Max iterations reached
      if (this.config.verbose) {
        console.log(
          `⚠️ Job ${job.jobId} reached maximum iterations (${maxIterations})`,
        );
      }

      return {
        jobId: job.jobId,
        success: false,
        error: `Job reached maximum iterations (${maxIterations}) without finishing`,
        result: allContent.join("\n\n--- Iteration ---\n\n"),
        metadata: {
          totalCostUsd: totalCost,
          ...(lastSessionId ? { sessionId: lastSessionId } : {}),
          iterations: iteration,
        },
      };
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
        metadata: {
          totalCostUsd: totalCost,
          ...(lastSessionId ? { sessionId: lastSessionId } : {}),
          iterations: iteration,
        },
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
        conversationHistory: llmRequest.conversationHistory,
        userInput: llmRequest.userInput,
      };

      if (this.config.verbose) {
        console.log(
          `📡 Sending to AI worker:`,
          JSON.stringify(requestData, null, 2),
        );
        console.log(`🌐 URL: ${this.config.workerUrl}/api/tonk`);
      }

      const response = await fetch(`${this.config.workerUrl}/api/tonk`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify(requestData),
        signal: controller.signal,
      });

      clearTimeout(timeoutId);

      if (this.config.verbose) {
        console.log(
          `📥 Response status: ${response.status} ${response.statusText}`,
        );
      }

      if (!response.ok) {
        const errorText = await response.text();
        if (this.config.verbose) {
          console.log(`❌ Error response body:`, errorText);
        }
        throw new Error(
          `HTTP ${response.status}: ${response.statusText} - ${errorText}`,
        );
      }

      const result = await response.json();

      if (this.config.verbose) {
        console.log(`📥 Response:`, JSON.stringify(result, null, 2));
      }

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
        const health: any = await response.json();
        return health.status === "ok";
      }

      return false;
    } catch (error) {
      return false;
    }
  }
}

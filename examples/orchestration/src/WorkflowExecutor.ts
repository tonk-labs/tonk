import { Job, JobRunner, JobRunnerConfig } from './JobRunner';
import { ContextManager, JobResult } from './ContextManager';

export interface WorkflowExecutorConfig extends JobRunnerConfig {
  maxConcurrentJobs?: number;
  continueOnFailure?: boolean;
}

export class WorkflowExecutor {
  private jobRunner: JobRunner;
  private contextManager: ContextManager;
  private config: WorkflowExecutorConfig;

  constructor(config: WorkflowExecutorConfig) {
    this.config = config;
    this.jobRunner = new JobRunner(config);
    this.contextManager = new ContextManager();
  }

  /**
   * Execute a complete workflow
   */
  async executeWorkflow(jobs: Job[]): Promise<{
    success: boolean;
    results: JobResult[];
    summary: ReturnType<ContextManager['getSummary']>;
  }> {
    if (this.config.verbose) {
      console.log(`🎯 Starting workflow execution with ${jobs.length} jobs`);
    }

    // Check worker health first
    if (this.config.verbose) {
      console.log(`🏥 Checking AI worker health...`);
    }
    const isHealthy = await this.jobRunner.checkWorkerHealth();
    if (!isHealthy) {
      throw new Error('AI worker is not healthy. Please ensure it is running and accessible.');
    }
    if (this.config.verbose) {
      console.log(`✅ AI worker is healthy`);
    }

    // Clear previous results
    this.contextManager.clear();

    // Build dependency graph and execution order
    const executionPlan = this.buildExecutionPlan(jobs);
    
    if (this.config.verbose) {
      console.log(`📋 Execution plan: ${executionPlan.map(batch => `[${batch.map(j => j.jobId).join(', ')}]`).join(' → ')}`);
    }

    // Execute jobs in batches based on dependencies
    let allSuccessful = true;
    
    for (const batch of executionPlan) {
      const batchResults = await this.executeBatch(batch);
      
      // Store results
      for (const result of batchResults) {
        this.contextManager.storeJobResult(result);
        if (!result.success) {
          allSuccessful = false;
        }
      }

      // Check if we should continue on failure
      if (!allSuccessful && !this.config.continueOnFailure) {
        if (this.config.verbose) {
          console.log('⚠️  Stopping workflow execution due to job failure');
        }
        break;
      }
    }

    const summary = this.contextManager.getSummary();
    const allResults = this.getAllResults();

    if (this.config.verbose) {
      console.log(`🏁 Workflow completed: ${summary.successfulJobs}/${summary.totalJobs} successful`);
      if (summary.totalCost > 0) {
        console.log(`💰 Total cost: $${summary.totalCost.toFixed(4)}`);
      }
    }

    return {
      success: allSuccessful,
      results: allResults,
      summary
    };
  }

  /**
   * Build execution plan based on job dependencies
   */
  private buildExecutionPlan(jobs: Job[]): Job[][] {
    const jobMap = new Map<number, Job>();
    const inDegree = new Map<number, number>();
    const dependents = new Map<number, number[]>();

    // Initialize maps
    for (const job of jobs) {
      jobMap.set(job.jobId, job);
      inDegree.set(job.jobId, job.dependencies?.length || 0);
      dependents.set(job.jobId, []);
    }

    // Build dependency graph
    for (const job of jobs) {
      if (job.dependencies) {
        for (const depId of job.dependencies) {
          const depDependents = dependents.get(depId) || [];
          depDependents.push(job.jobId);
          dependents.set(depId, depDependents);
        }
      }
    }

    // Topological sort to create execution batches
    const batches: Job[][] = [];
    const remaining = new Set(jobs.map(j => j.jobId));
    
    while (remaining.size > 0) {
      const currentBatch: Job[] = [];
      
      // Find jobs with no remaining dependencies
      for (const jobId of remaining) {
        if (inDegree.get(jobId) === 0) {
          currentBatch.push(jobMap.get(jobId)!);
        }
      }

      if (currentBatch.length === 0) {
        throw new Error('Circular dependency detected in workflow');
      }

      // Remove processed jobs and update dependencies
      for (const job of currentBatch) {
        remaining.delete(job.jobId);
        
        // Update dependent jobs
        const jobDependents = dependents.get(job.jobId) || [];
        for (const dependentId of jobDependents) {
          const currentInDegree = inDegree.get(dependentId) || 0;
          inDegree.set(dependentId, currentInDegree - 1);
        }
      }

      batches.push(currentBatch);
    }

    return batches;
  }

  /**
   * Execute a batch of jobs concurrently
   */
  private async executeBatch(jobs: Job[]): Promise<JobResult[]> {
    if (this.config.verbose && jobs.length > 1) {
      console.log(`⚡ Executing batch of ${jobs.length} jobs concurrently: [${jobs.map(j => j.jobId).join(', ')}]`);
    }

    const maxConcurrent = this.config.maxConcurrentJobs || 3;
    const results: JobResult[] = [];

    // Execute jobs in chunks to limit concurrency
    for (let i = 0; i < jobs.length; i += maxConcurrent) {
      const chunk = jobs.slice(i, i + maxConcurrent);
      const chunkPromises = chunk.map(job => this.executeJobWithContext(job));
      const chunkResults = await Promise.allSettled(chunkPromises);
      
      for (const result of chunkResults) {
        if (result.status === 'fulfilled') {
          results.push(result.value);
        } else {
          // Handle rejected promises
          console.error('Job execution failed:', result.reason);
          results.push({
            jobId: -1, // We don't know which job failed
            success: false,
            error: result.reason instanceof Error ? result.reason.message : String(result.reason)
          });
        }
      }
    }

    return results;
  }

  /**
   * Execute a job with context from its dependencies
   */
  private async executeJobWithContext(job: Job): Promise<JobResult> {
    let context = '';
    
    if (job.dependencies && job.dependencies.length > 0) {
      // Check if all dependencies are complete
      if (!this.contextManager.areDependenciesComplete(job.dependencies)) {
        return {
          jobId: job.jobId,
          success: false,
          error: 'Dependencies not completed successfully'
        };
      }
      
      context = this.contextManager.buildContextFromDependencies(job.dependencies);
    }

    return await this.jobRunner.executeJob(job, context);
  }

  /**
   * Get all job results
   */
  private getAllResults(): JobResult[] {
    return [...this.contextManager.getSuccessfulResults(), ...this.contextManager.getFailedResults()];
  }

  /**
   * Get context manager for external access
   */
  getContextManager(): ContextManager {
    return this.contextManager;
  }
}
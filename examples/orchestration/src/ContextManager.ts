export interface JobResult {
  jobId: number;
  success: boolean;
  result?: string;
  error?: string;
  metadata?: {
    totalCostUsd?: number;
    sessionId?: string;
    provider?: string;
  };
}

export class ContextManager {
  private jobResults = new Map<number, JobResult>();

  /**
   * Store the result of a completed job
   */
  storeJobResult(result: JobResult): void {
    this.jobResults.set(result.jobId, result);
  }

  /**
   * Get the result of a specific job
   */
  getJobResult(jobId: number): JobResult | undefined {
    return this.jobResults.get(jobId);
  }

  /**
   * Get results for multiple jobs
   */
  getJobResults(jobIds: number[]): JobResult[] {
    return jobIds.map(id => this.jobResults.get(id)).filter(result => result !== undefined) as JobResult[];
  }

  /**
   * Build context string from dependency results
   */
  buildContextFromDependencies(dependencyIds: number[]): string {
    const results = this.getJobResults(dependencyIds);
    
    if (results.length === 0) {
      return '';
    }

    const contextParts: string[] = [];
    
    for (const result of results) {
      if (result.success && result.result) {
        contextParts.push(`=== Job ${result.jobId} Output ===`);
        contextParts.push(result.result);
        contextParts.push(''); // Empty line for separation
      }
    }

    return contextParts.join('\n');
  }

  /**
   * Get all successful job results
   */
  getSuccessfulResults(): JobResult[] {
    return Array.from(this.jobResults.values()).filter(result => result.success);
  }

  /**
   * Get all failed job results
   */
  getFailedResults(): JobResult[] {
    return Array.from(this.jobResults.values()).filter(result => !result.success);
  }

  /**
   * Check if all dependencies are completed successfully
   */
  areDependenciesComplete(dependencyIds: number[]): boolean {
    for (const depId of dependencyIds) {
      const result = this.jobResults.get(depId);
      if (!result || !result.success) {
        return false;
      }
    }
    return true;
  }

  /**
   * Get a summary of all job results
   */
  getSummary(): {
    totalJobs: number;
    successfulJobs: number;
    failedJobs: number;
    totalCost: number;
  } {
    const allResults = Array.from(this.jobResults.values());
    const successfulJobs = allResults.filter(r => r.success).length;
    const failedJobs = allResults.filter(r => !r.success).length;
    const totalCost = allResults.reduce((sum, r) => sum + (r.metadata?.totalCostUsd || 0), 0);

    return {
      totalJobs: allResults.length,
      successfulJobs,
      failedJobs,
      totalCost
    };
  }

  /**
   * Clear all stored results
   */
  clear(): void {
    this.jobResults.clear();
  }
}
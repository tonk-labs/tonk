/**
 * ⚠️  CORE TEMPLATE FILE - DO NOT DELETE! ⚠️
 *
 * This file is part of the Tonk worker template infrastructure.
 * It provides file system watching capabilities for workers that need to
 * monitor file changes and automatically process them into keepsync documents.
 *
 * USAGE: Import and use this when you need to:
 * - Watch local files for changes
 * - Automatically process file content when files are added/modified
 * - Transform file data and store it in keepsync
 *
 * Even if your current worker doesn't use file watching, other workers might!
 * Keep this file for the template integrity.
 */
import { writeDoc, readDoc } from "@tonk/keepsync";
import { readFileSync } from "fs";
import { join } from "path";

interface LogEntry {
  type: "read" | "write";
  source: string;
  uri: string;
  timestamp: string;
  data: any;
  "worker-name": string;
}

interface LogDocument {
  workers: {
    [workerName: string]: {
      [sourceUri: string]: LogEntry[];
    };
  };
}

export class KeepsyncLogger {
  private readonly logPath = "/console/logs/worker";
  private workerName: string;
  private readonly maxEntriesPerSourceUri = 5;

  constructor() {
    this.workerName = this.getWorkerName();
  }

  /**
   * Get the worker name from package.json
   */
  private getWorkerName(): string {
    try {
      const packageJsonPath = join(__dirname, "../../package.json");
      const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8"));
      return packageJson.name || "unknown-worker";
    } catch (error) {
      console.error("Failed to read worker name from package.json:", error);
      return "unknown-worker";
    }
  }

  /**
   * Get current log document from keepsync
   */
  private async getCurrentLogDocument(): Promise<LogDocument> {
    try {
      const logDoc = await readDoc<LogDocument>(this.logPath);
      return logDoc || { workers: {} };
    } catch (error) {
      console.error("Failed to read existing logs:", error);
      return { workers: {} };
    }
  }

  /**
   * Write logs back to keepsync
   */
  private async writeLogDocument(logDoc: LogDocument): Promise<void> {
    try {
      await writeDoc(this.logPath, logDoc);
    } catch (error) {
      console.error("Failed to write logs to keepsync:", error);
      throw error;
    }
  }

  /**
   * Create a source:uri key for grouping logs
   */
  private createSourceUriKey(source: string, uri: string): string {
    return `${source}:${uri}`;
  }

  /**
   * Add a log entry and maintain the max entries limit
   */
  private async addLogEntry(logEntry: LogEntry): Promise<void> {
    const logDoc = await this.getCurrentLogDocument();

    // Initialize worker if it doesn't exist
    if (!logDoc.workers[this.workerName]) {
      logDoc.workers[this.workerName] = {};
    }

    const sourceUriKey = this.createSourceUriKey(logEntry.source, logEntry.uri);

    // Initialize source:uri array if it doesn't exist
    if (!logDoc.workers[this.workerName][sourceUriKey]) {
      logDoc.workers[this.workerName][sourceUriKey] = [];
    }

    // Add the new entry
    logDoc.workers[this.workerName][sourceUriKey].push(logEntry);

    // Keep only the last N entries
    if (
      logDoc.workers[this.workerName][sourceUriKey].length >
      this.maxEntriesPerSourceUri
    ) {
      logDoc.workers[this.workerName][sourceUriKey] = logDoc.workers[
        this.workerName
      ][sourceUriKey].slice(-this.maxEntriesPerSourceUri);
    }

    await this.writeLogDocument(logDoc);
  }

  /**
   * Log a read operation
   * @param source - The source type of the read operation (e.g., 'web', 'fs', 'keepsync')
   * @param path - The specific path/location that was read
   * @param data - The data that was read
   */
  async logRead(source: string, path: string, data: any): Promise<void> {
    const logEntry: LogEntry = {
      type: "read",
      source,
      uri: path,
      timestamp: new Date().toISOString(),
      data,
      "worker-name": this.workerName,
    };

    try {
      await this.addLogEntry(logEntry);
    } catch (error) {
      console.error("Failed to log read operation:", error);
      throw error;
    }
  }

  /**
   * Log a write operation
   * @param source - The source type of the write operation (e.g., 'web', 'fs', 'keepsync')
   * @param path - The specific path/location that was written to
   * @param data - The data that was written
   */
  async logWrite(source: string, path: string, data: any): Promise<void> {
    const logEntry: LogEntry = {
      type: "write",
      source,
      uri: path,
      timestamp: new Date().toISOString(),
      data,
      "worker-name": this.workerName,
    };

    try {
      await this.addLogEntry(logEntry);
    } catch (error) {
      console.error("Failed to log write operation:", error);
      throw error;
    }
  }

  /**
   * Get all logs for this worker (useful for debugging)
   */
  async getLogs(): Promise<LogEntry[]> {
    try {
      const logDoc = await this.getCurrentLogDocument();
      const workerLogs = logDoc.workers[this.workerName];

      if (!workerLogs) {
        return [];
      }

      // Flatten all source:uri entries into a single array
      const allLogs: LogEntry[] = [];
      for (const sourceUriKey in workerLogs) {
        allLogs.push(...workerLogs[sourceUriKey]);
      }

      // Sort by timestamp
      return allLogs.sort(
        (a, b) =>
          new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime()
      );
    } catch (error) {
      console.error("Failed to get logs:", error);
      return [];
    }
  }

  /**
   * Get logs for a specific source:uri combination
   */
  async getLogsForSourceUri(source: string, uri: string): Promise<LogEntry[]> {
    try {
      const logDoc = await this.getCurrentLogDocument();
      const sourceUriKey = this.createSourceUriKey(source, uri);

      return logDoc.workers[this.workerName]?.[sourceUriKey] || [];
    } catch (error) {
      console.error("Failed to get logs for source:uri:", error);
      return [];
    }
  }

  /**
   * Clear all logs for this worker
   */
  async clearLogs(): Promise<void> {
    try {
      const logDoc = await this.getCurrentLogDocument();
      delete logDoc.workers[this.workerName];
      await this.writeLogDocument(logDoc);
    } catch (error) {
      console.error("Failed to clear logs:", error);
      throw error;
    }
  }

  /**
   * Clear logs for a specific source:uri combination
   */
  async clearLogsForSourceUri(source: string, uri: string): Promise<void> {
    try {
      const logDoc = await this.getCurrentLogDocument();
      const sourceUriKey = this.createSourceUriKey(source, uri);

      if (logDoc.workers[this.workerName]?.[sourceUriKey]) {
        delete logDoc.workers[this.workerName][sourceUriKey];
        await this.writeLogDocument(logDoc);
      }
    } catch (error) {
      console.error("Failed to clear logs for source:uri:", error);
      throw error;
    }
  }
}

// Export a singleton instance for easy usage
export const keepsyncLogger = new KeepsyncLogger();

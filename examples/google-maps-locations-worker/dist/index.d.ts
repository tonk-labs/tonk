import * as http from "http";
/**
 * Configuration for the worker
 */
interface WorkerConfig {
    port: number;
    keepsyncDocPath?: string;
}
/**
 * Start the worker with the given configuration
 */
export declare function startWorker(config: WorkerConfig): Promise<http.Server>;
export {};

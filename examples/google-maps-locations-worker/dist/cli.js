#!/usr/bin/env node
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
/**
 * CLI entry point for the worker
 */
const commander_1 = require("commander");
const index_1 = require("./index");
const program = new commander_1.Command();
program
    .name("google-maps-locations-worker")
    .description("Retrieves user's saved Google Maps locations")
    .version("1.0.0");
program
    .command("start")
    .description("Start the worker")
    .option("-p, --port <port>", "Port to run the worker on", process.env.WORKER_PORT || "5555")
    .option("-k, --keepsync-doc-path <path>", "Path to store locations in Keepsync", process.env.KEEPSYNC_DOC_PATH)
    .action(async (options) => {
    try {
        console.log(`Starting google-maps-locations-worker worker on port ${options.port}...`);
        await (0, index_1.startWorker)({
            port: parseInt(options.port, 10),
            keepsyncDocPath: options.keepsyncDocPath,
        });
        console.log(`google-maps-locations-worker worker is running`);
    }
    catch (error) {
        console.error("Failed to start worker:", error);
        process.exit(1);
    }
});
program.parse(process.argv);

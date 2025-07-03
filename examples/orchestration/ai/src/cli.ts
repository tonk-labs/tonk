#!/usr/bin/env node

/**
 * CLI entry point for the worker
 */
import { Command } from "commander";
import { startWorker } from "./index";

const program = new Command();

program.name("ai").description("Query Claude Code").version("1.0.0");

program
  .command("start")
  .description("Start the worker")
  .option(
    "-p, --port <port>",
    "Port to run the worker on",
    process.env.WORKER_PORT || "5556",
  )
  .action(async (options) => {
    try {
      console.log(`Starting ai worker on port ${options.port}...`);
      await startWorker({
        port: parseInt(options.port, 10),
      });
      console.log(`ai worker is running`);
    } catch (error) {
      console.error("Failed to start worker:", error);
      process.exit(1);
    }
  });

// Show help if no command provided
if (process.argv.length <= 2) {
  program.help();
}

program.parse(process.argv);

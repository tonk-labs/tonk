#!/usr/bin/env node

/**
 * CLI entry point for the worker
 */
import { Command } from "commander";
import { startWorker } from "./index";

const program = new Command();

program.name("{{name}}").description("{{description}}").version("{{version}}");

program
  .command("start")
  .description("Start the worker")
  .option(
    "-p, --port <port>",
    "Port to run the worker on",
    process.env.WORKER_PORT || "5555",
  )
  .action(async (options) => {
    try {
      console.log(`Starting {{name}} worker on port ${options.port}...`);
      await startWorker({
        port: parseInt(options.port, 10),
      });
      console.log(`{{name}} worker is running`);
    } catch (error) {
      console.error("Failed to start worker:", error);
      process.exit(1);
    }
  });

program.parse(process.argv);

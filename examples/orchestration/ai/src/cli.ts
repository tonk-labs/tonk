#!/usr/bin/env node

/**
 * CLI entry point for the worker
 */
import { Command } from "commander";
import { startWorker } from "./index";
import { BaseCredentialsManager } from "./utils/baseCredentialsManager";

const program = new Command();

program.name("ai").description("Query the OpenAI API").version("1.0.0");

program
  .command("start")
  .description("Start the worker")
  .option(
    "-p, --port <port>",
    "Port to run the worker on",
    process.env.WORKER_PORT || "5556",
  )
  .option(
    "-c, --chroma-port <port>",
    "Port to run Chroma server on",
    process.env.CHROMA_PORT || "8888",
  )
  .action(async (options) => {
    try {
      console.log(`Starting ai worker on port ${options.port}...`);
      console.log(`Chroma will run on port ${options.chromaPort}...`);
      await startWorker({
        port: parseInt(options.port, 10),
        chromaPort: parseInt(options.chromaPort, 10),
      });
      console.log(`ai worker is running`);
    } catch (error) {
      console.error("Failed to start worker:", error);
      process.exit(1);
    }
  });

program
  .command("setup")
  .description("Set up credentials for external services")
  .action(async () => {
    try {
      const credentialsManager = new BaseCredentialsManager(
        [
          {
            name: "OpenAI API Key",
            filename: "creds/openai_api_key.txt",
            description: "OpenAI API key for accessing GPT models",
            instructions:
              "Get your API key from https://platform.openai.com/api-keys",
            validationFn: (content) => {
              const trimmed = content.trim();
              return {
                valid: trimmed.startsWith("sk-") && trimmed.length > 20,
                message: "Must be a valid OpenAI API key starting with 'sk-'",
              };
            },
            header: "Authorization",
          },
        ],
        process.cwd(),
      );

      await credentialsManager.init();
      await credentialsManager.setupCredentials();

      console.log(
        "\n🎉 Setup complete! You can now start the worker with 'pnpm start'.",
      );
    } catch (error) {
      console.error("Setup failed:", error);
      process.exit(1);
    }
  });

// Show help if no command provided
if (process.argv.length <= 2) {
  program.help();
}

program.parse(process.argv);

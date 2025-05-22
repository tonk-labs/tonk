#!/usr/bin/env node

/**
 * CLI entry point for the worker
 */
import { Command } from "commander";
import { startWorker } from "./index";
import { createGoogleOAuthCredentialsManager } from "./credentialsManager";
import { main as exportLocations } from "./exportLocations";

const program = new Command();

program
  .name("google-maps-locations-worker")
  .description("Retrieves user's saved Google Maps locations")
  .version("1.0.0");

program
  .command("setup")
  .description("Set up required credentials and authentication for the worker")
  .action(async () => {
    try {
      console.log("Setting up Google Maps Locations Worker...");

      // Create and use the credentials manager
      const credentialsManager = createGoogleOAuthCredentialsManager();

      // Step 1: Set up credentials.json
      console.log("\n📋 Step 1: Setting up credentials.json");
      await credentialsManager.setupCredentials();

      // Step 2: Run OAuth authentication to get token.json
      console.log("\n📋 Step 2: Setting up OAuth authentication");

      // Check if credentials.json exists before proceeding
      const { complete } = credentialsManager.checkCredentials();

      if (!complete) {
        console.log(
          "⚠️ Cannot proceed with OAuth authentication because credentials.json is missing.",
        );
        console.log(
          "Please add the credentials.json file manually and then run 'google-maps-locations-worker setup' again.",
        );
        return;
      }

      console.log("Running OAuth authentication to get token.json...");
      console.log(
        "This will open a browser window for you to authenticate with Google.",
      );
      console.log(
        "After authentication, a token.json file will be created with refresh credentials.",
      );

      // Run the exportLocations function which will trigger the OAuth flow
      try {
        await exportLocations();
        console.log("\n✅ OAuth authentication completed successfully!");
        console.log(
          "A token.json file has been created with refresh credentials.",
        );
      } catch (error) {
        console.error("Error during OAuth authentication:", error);
        console.log(
          "\n⚠️ OAuth authentication failed. You can try again later by running:",
        );
        console.log("google-maps-locations-worker setup");
      }

      console.log("\n🎉 Setup complete!");
      console.log("\nNext steps:");
      console.log(
        "Run 'google-maps-locations-worker start' to start the worker",
      );

      process.exit(0);
    } catch (error) {
      console.error("Failed to set up:", error);
      process.exit(1);
    }
  });

program
  .command("export")
  .description("Run a one-time export of Google Maps locations")
  .action(async () => {
    try {
      console.log("Running one-time export of Google Maps locations...");
      await exportLocations();
      console.log("Export completed successfully!");
    } catch (error) {
      console.error("Failed to export locations:", error);
      process.exit(1);
    }
  });

program
  .command("start")
  .description("Start the worker")
  .option(
    "-p, --port <port>",
    "Port to run the worker on",
    process.env.WORKER_PORT || "5555",
  )
  .option(
    "-k, --keepsync-doc-path <path>",
    "Path to store locations in Keepsync",
    process.env.KEEPSYNC_DOC_PATH,
  )
  .action(async (options) => {
    try {
      // Check for required credentials before starting
      const credentialsManager = createGoogleOAuthCredentialsManager();
      const { complete, missing } = credentialsManager.checkCredentials();

      if (!complete) {
        console.log("⚠️ Missing required credentials:", missing.join(", "));
        console.log(
          "Please run 'google-maps-locations-worker setup' to set up credentials",
        );
        process.exit(1);
      }

      console.log(
        `Starting google-maps-locations-worker worker on port ${options.port}...`,
      );
      await startWorker({
        port: parseInt(options.port, 10),
        keepsyncDocPath: options.keepsyncDocPath,
      });
      console.log(`google-maps-locations-worker worker is running`);
    } catch (error) {
      console.error("Failed to start worker:", error);
      process.exit(1);
    }
  });

program.parse(process.argv);

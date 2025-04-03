#!/usr/bin/env node

// Import the CLI modules
import { program } from "commander";
import * as dotenv from "dotenv";
import { convertLocations } from "./convert/convertLocations";
import { runOAuthFlow } from "./auth/oauthCli";

// Load environment variables
dotenv.config();

// Set up the main program with subcommands
program
  .name("google-maps-locations")
  .description("Google Maps Saved Locations CLI")
  .version("0.1.0");

// Add the convert command (existing functionality)
program
  .command("convert")
  .description("Convert CSV locations to Google Maps data")
  .option("-i, --input <path>", "Path to input CSV file", "locations.csv")
  .option(
    "-o, --output <path>",
    "Path to output JSON file",
    "locations-output.json",
  )
  .option(
    "-k, --api-key <key>",
    "Google Maps API key (or set GOOGLE_MAPS_API_KEY env var)",
  )
  .action(async (options) => {
    try {
      await convertLocations({
        apiKey: options.apiKey,
        inputCsvPath: options.input,
        outputJsonPath: options.output,
      });
      console.log("Conversion completed successfully");
    } catch (error) {
      console.error("Conversion failed:", error);
      process.exit(1);
    }
  });

// Add the oauth command (new functionality)
program
  .command("oauth")
  .description("Fetch your saved Google Maps locations using OAuth")
  .option(
    "-i, --client-id <id>",
    "Google OAuth Client ID (or set GOOGLE_OAUTH_CLIENT_ID env var)",
  )
  .option(
    "-s, --client-secret <secret>",
    "Google OAuth Client Secret (or set GOOGLE_OAUTH_CLIENT_SECRET env var)",
  )
  .option(
    "-o, --output <path>",
    "Path to output JSON file",
    "locations-output.json",
  )
  .action(async (options) => {
    try {
      // Get credentials from options or environment variables
      const clientId = options.clientId || process.env.GOOGLE_OAUTH_CLIENT_ID;
      const clientSecret =
        options.clientSecret || process.env.GOOGLE_OAUTH_CLIENT_SECRET;

      if (!clientId || !clientSecret) {
        console.error(
          "Error: Google OAuth credentials are required.\n" +
            "Provide them using --client-id and --client-secret options " +
            "or set GOOGLE_OAUTH_CLIENT_ID and GOOGLE_OAUTH_CLIENT_SECRET environment variables.",
        );
        process.exit(1);
      }

      await runOAuthFlow({
        clientId,
        clientSecret,
        outputPath: options.output,
      });

      console.log("Process completed successfully");
    } catch (error) {
      console.error("Process failed:", error);
      process.exit(1);
    }
  });

// If no command is specified, show help
if (process.argv.length <= 2) {
  program.help();
} else {
  program.parse();
}
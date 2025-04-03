#!/usr/bin/env node
import * as dotenv from "dotenv";
import { program } from "commander";
import { convertLocations } from "./convertLocations";
import { SchemaMapping, DEFAULT_SCHEMA_MAPPING } from "./schema";

dotenv.config();

// Parse schema mapping from command line
function parseSchemaMapping(value: string): SchemaMapping {
  try {
    return JSON.parse(value);
  } catch (error) {
    console.error("Invalid schema mapping JSON:", error);
    return DEFAULT_SCHEMA_MAPPING;
  }
}

program
  .name("convert-locations")
  .description("Convert CSV locations to Google Maps data and merge into an Automerge document")
  .option("-i, --input <path>", "Path to input CSV file", "locations.csv")
  .option("-o, --output <path>", "Path to output Automerge binary file", "locations.bin")
  .option("-j, --json <path>", "Path to save intermediate JSON output")
  .option("-k, --api-key <key>", "Google Maps API key (or set GOOGLE_MAPS_API_KEY env var)")
  .option("-s, --schema <json>", "Schema mapping as JSON string", parseSchemaMapping, DEFAULT_SCHEMA_MAPPING)
  .option("-u, --user-id <id>", "Default user ID for imported locations", "7fn52mcm1f5")
  .option("-n, --user-name <name>", "Default user name", "Jack")
  .action(async (options) => {
    try {
      await convertLocations({
        apiKey: options.apiKey,
        inputCsvPath: options.input,
        outputAutomergePath: options.output,
        outputJsonPath: options.json,
        schemaMapping: options.schema,
        defaultUserId: options.userId,
        defaultUserName: options.userName,
      });
      console.log("Conversion completed successfully");
    } catch (error) {
      console.error("Conversion failed:", error);
      process.exit(1);
    }
  });

program.parse();

import * as dotenv from "dotenv";
import { program } from "commander";
import { convertLocations } from "./convertLocations";

dotenv.config();

program
  .name("google-maps-locations")
  .description("Convert CSV locations to Google Maps data and output as JSON")
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

program.parse();

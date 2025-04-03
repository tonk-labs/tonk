import { parse } from "csv-parse/sync";
import * as fs from "fs/promises";
import * as dotenv from "dotenv";
import { ConversionConfig, createConfig } from "./config";
import { transformCsvToLocations } from "./transformer";

dotenv.config();

/**
 * Main conversion function that processes a CSV file and outputs location data as JSON
 * @param configOptions Configuration options for the conversion
 */
export async function convertLocations(
  configOptions: Partial<ConversionConfig> = {},
): Promise<void> {
  // Create complete configuration with defaults
  const config = createConfig(configOptions);

  try {
    // Read and parse CSV file
    const csvContent = await fs.readFile(config.inputCsvPath, "utf-8");
    const records = parse(csvContent, {
      columns: true,
      skip_empty_lines: true,
    });

    // Transform CSV records to location data
    const output = await transformCsvToLocations(records, config.apiKey);

    // Save JSON output
    const outputPath = config.outputJsonPath || "locations-output.json";
    await fs.writeFile(outputPath, JSON.stringify(output, null, 2));
    console.log(`Saved JSON output to ${outputPath}`);

    return;
  } catch (error) {
    console.error("Error during conversion:", error);
    throw error;
  }
}

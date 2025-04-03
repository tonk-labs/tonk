import { parse } from "csv-parse/sync";
import * as fs from "fs/promises";
import * as dotenv from "dotenv";
import { ConversionConfig, createConfig } from "./config";
import { transformCsvToLocations } from "./transformer";
import { loadAutomergeDoc, saveAutomergeDoc } from "./automergeUtils";
import { mergeLocationsIntoDoc } from "./mergeStrategy";

dotenv.config();

/**
 * Main conversion function that processes a CSV file and merges the data into an Automerge document
 * @param configOptions Configuration options for the conversion
 */
export async function convertLocations(configOptions: Partial<ConversionConfig> = {}): Promise<void> {
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
    const output = await transformCsvToLocations(
      records, 
      config.apiKey,
      config.defaultUserId,
      config.defaultUserName
    );

    // Optionally save JSON output for debugging
    if (config.outputJsonPath) {
      await fs.writeFile(config.outputJsonPath, JSON.stringify(output, null, 2));
      console.log(`Saved JSON output to ${config.outputJsonPath}`);
    }

    // Load or create Automerge document
    console.log(`Loading Automerge document from ${config.outputAutomergePath}`);
    let doc = await loadAutomergeDoc(config.outputAutomergePath);

    // Merge location data into the document using the specified schema mapping
    doc = mergeLocationsIntoDoc(doc, output, config.schemaMapping);

    // Save the updated Automerge document
    await saveAutomergeDoc(doc, config.outputAutomergePath);
    console.log(`Updated Automerge document saved to ${config.outputAutomergePath}`);
    
    return;
  } catch (error) {
    console.error("Error during conversion:", error);
    throw error;
  }
}

import { SchemaMapping, DEFAULT_SCHEMA_MAPPING } from "./schema";

/**
 * Configuration for the conversion process
 */
export interface ConversionConfig {
  // Google Maps API key
  apiKey: string;
  // Path to the input CSV file
  inputCsvPath: string;
  // Path to the output Automerge binary file
  outputAutomergePath: string;
  // Optional path to save JSON output for debugging
  outputJsonPath?: string;
  // Schema mapping configuration
  schemaMapping: SchemaMapping;
  // Default user ID for imported locations
  defaultUserId?: string;
  // Default user name for the default user ID
  defaultUserName?: string;
}

/**
 * Default configuration values
 */
export const DEFAULT_CONFIG: Partial<ConversionConfig> = {
  inputCsvPath: "locations.csv",
  outputAutomergePath: "locations.bin",
  outputJsonPath: "output.json",
  schemaMapping: DEFAULT_SCHEMA_MAPPING,
  defaultUserId: "7fn52mcm1f5",
  defaultUserName: "Jack",
};

/**
 * Creates a complete configuration by merging provided options with defaults
 * @param options Partial configuration options
 * @returns Complete configuration with defaults applied
 */
export function createConfig(options: Partial<ConversionConfig>): ConversionConfig {
  // Ensure API key is provided
  if (!options.apiKey && !process.env.GOOGLE_MAPS_API_KEY) {
    throw new Error("Google Maps API key must be provided in config or as GOOGLE_MAPS_API_KEY environment variable");
  }
  
  return {
    ...DEFAULT_CONFIG,
    ...options,
    apiKey: options.apiKey || process.env.GOOGLE_MAPS_API_KEY!,
    schemaMapping: {
      ...DEFAULT_SCHEMA_MAPPING,
      ...options.schemaMapping,
    },
  } as ConversionConfig;
}

/**
 * Configuration for the conversion process
 */
export interface ConversionConfig {
  // Google Maps API key
  apiKey: string;
  // Path to the input CSV file
  inputCsvPath: string;
  // Optional path to save JSON output
  outputJsonPath?: string;
}

/**
 * Default configuration values
 */
export const DEFAULT_CONFIG: Partial<ConversionConfig> = {
  inputCsvPath: "locations.csv",
  outputJsonPath: "locations-output.json",
};

/**
 * Creates a complete configuration by merging provided options with defaults
 * @param options Partial configuration options
 * @returns Complete configuration with defaults applied
 */
export function createConfig(
  options: Partial<ConversionConfig>,
): ConversionConfig {
  // Ensure API key is provided
  if (!options.apiKey && !process.env.GOOGLE_MAPS_API_KEY) {
    throw new Error(
      "Google Maps API key must be provided in config or as GOOGLE_MAPS_API_KEY environment variable",
    );
  }

  return {
    ...DEFAULT_CONFIG,
    ...options,
    apiKey: options.apiKey || process.env.GOOGLE_MAPS_API_KEY!,
  } as ConversionConfig;
}

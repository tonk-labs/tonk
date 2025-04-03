# Google Maps Location Converter

This module provides a flexible system for converting CSV location data into Google Maps location data and merging it into an Automerge document with customizable schema mapping.

## Features

- Convert CSV location data to Google Maps location data
- Flexible schema mapping for different Automerge document structures
- Command-line interface for batch processing
- TUI integration for interactive use
- Custom transformation functions for advanced use cases

## Usage

### Basic Usage

```typescript
import { convertLocations } from './convert';

// Use default configuration (reads from .env for API key)
await convertLocations();
```

### Custom Schema Mapping

```typescript
import { convertLocations, SchemaMapping } from './convert';

// Define a custom schema mapping
const customMapping: SchemaMapping = {
  locationsPath: ['data', 'places'],
  userNamesPath: ['data', 'users'],
  // Optional transformation functions
  transformLocation: (location) => ({
    ...location,
    customField: 'value',
  }),
  transformUserName: (userId, userName) => ({
    id: userId,
    name: userName,
    role: 'user',
  }),
};

// Use custom configuration
await convertLocations({
  apiKey: 'your-google-maps-api-key',
  inputCsvPath: 'custom-locations.csv',
  outputAutomergePath: 'custom-output.bin',
  schemaMapping: customMapping,
});
```

### Command Line Interface

```bash
# Basic usage
npx ts-node src/convert/cli.ts

# Custom options
npx ts-node src/convert/cli.ts \
  --input custom-locations.csv \
  --output custom-output.bin \
  --api-key YOUR_API_KEY \
  --schema '{"locationsPath":["data","places"],"userNamesPath":["data","users"]}'
```

## Configuration Options

| Option | Description | Default |
|--------|-------------|--------|
| `apiKey` | Google Maps API key | From GOOGLE_MAPS_API_KEY env var |
| `inputCsvPath` | Path to input CSV file | "locations.csv" |
| `outputAutomergePath` | Path to output Automerge binary file | "locations.bin" |
| `outputJsonPath` | Path to save intermediate JSON output | "output.json" |
| `schemaMapping` | Schema mapping configuration | DEFAULT_SCHEMA_MAPPING |
| `defaultUserId` | Default user ID for imported locations | "7fn52mcm1f5" |
| `defaultUserName` | Default user name | "Jack" |

## CSV Format

The input CSV file should have the following columns:

- `Title`: The name of the location (required)
- `Note`: Description of the location (optional)

## Schema Mapping

The schema mapping allows you to customize how the location data is stored in your Automerge document:

```typescript
interface SchemaMapping {
  // Path to where locations should be stored in the document
  locationsPath: string[];
  // Path to where userNames should be stored in the document
  userNamesPath: string[];
  // Optional transformation function for location objects
  transformLocation?: (location: Location) => any;
  // Optional transformation function for userName objects
  transformUserName?: (userId: string, userName: string) => any;
}
```

The default schema mapping stores locations at `doc.locations` and user names at `doc.userNames`.

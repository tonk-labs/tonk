# Google Maps Saved Locations Converter

A tool to convert CSV files containing location data into a structured JSON format with enhanced data from the Google Maps API.

## Features

- Reads location data from CSV files
- Enriches location data using the Google Maps API
- Outputs structured JSON data
- Simple command-line interface

## Installation

```bash
npm install -g google-maps-saved-locations
```

Or use it directly with npx:

```bash
npx google-maps-saved-locations --input your-locations.csv --output enriched-locations.json
```

## Usage

```bash
google-maps-locations --input your-locations.csv --output enriched-locations.json --api-key YOUR_GOOGLE_MAPS_API_KEY
```

### Options

- `-i, --input <path>`: Path to input CSV file (default: "locations.csv")
- `-o, --output <path>`: Path to output JSON file (default: "locations-output.json")
- `-k, --api-key <key>`: Google Maps API key (or set GOOGLE_MAPS_API_KEY env var)
- `-u, --user-id <id>`: Default user ID for imported locations (default: "7fn52mcm1f5")
- `-n, --user-name <name>`: Default user name (default: "Jack")

## CSV Format

The CSV file should have the following columns:

- `Title`: The name of the location
- `Note`: Optional description or notes about the location

## Output Format

The tool outputs a JSON file with the following structure:

```json
{
  "locations": {
    "locationId1": {
      "addedBy": "userId",
      "category": "favorite",
      "createdAt": 1617235200000,
      "description": "Location description",
      "id": "locationId1",
      "latitude": 37.7749,
      "longitude": -122.4194,
      "name": "Location Name",
      "placeId": "googlePlaceId"
    },
    // More locations...
  },
  "userNames": {
    "userId": "User Name",
    // More users...
  }
}
```

## Environment Variables

You can set the following environment variables instead of using command-line options:

- `GOOGLE_MAPS_API_KEY`: Your Google Maps API key

## License

MIT

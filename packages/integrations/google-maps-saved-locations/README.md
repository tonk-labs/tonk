# Google Maps Saved Locations

A tool to work with Google Maps saved locations. It provides two main functionalities:

1. **OAuth Flow**: Fetch your saved locations directly from Google Maps using OAuth authentication
2. **CSV Converter**: Convert CSV files containing location data into a structured JSON format with enhanced data from the Google Maps API

## Features

- Authenticate with Google using OAuth 2.0
- Retrieve your saved locations from Google Maps via the Data Portability API
- Read location data from CSV files
- Enrich location data using the Google Maps API
- Output structured JSON data
- Simple command-line interface

## Installation

```bash
npm install -g google-maps-saved-locations
```

Or use it directly with npx:

```bash
npx google-maps-saved-locations
```

## Usage

The CLI provides two main commands:

### OAuth Flow

Fetch your saved locations directly from Google Maps:

```bash
google-maps-locations oauth --client-id YOUR_GOOGLE_CLIENT_ID --client-secret YOUR_GOOGLE_CLIENT_SECRET --output my-locations.json
```

#### OAuth Options

- `-i, --client-id <id>`: Google OAuth Client ID (or set GOOGLE_OAUTH_CLIENT_ID env var)
- `-s, --client-secret <secret>`: Google OAuth Client Secret (or set GOOGLE_OAUTH_CLIENT_SECRET env var)
- `-o, --output <path>`: Path to output JSON file (default: "locations-output.json")

### CSV Converter

Convert CSV files containing location data:

```bash
google-maps-locations convert --input your-locations.csv --output enriched-locations.json --api-key YOUR_GOOGLE_MAPS_API_KEY
```

#### Convert Options

- `-i, --input <path>`: Path to input CSV file (default: "locations.csv")
- `-o, --output <path>`: Path to output JSON file (default: "locations-output.json")
- `-k, --api-key <key>`: Google Maps API key (or set GOOGLE_MAPS_API_KEY env var)

## Setting Up Google OAuth Credentials

To use the OAuth flow, you need to set up a Google Cloud project and create OAuth credentials:

1. Go to the [Google Cloud Console](https://console.cloud.google.com/)
2. Create a new project or select an existing one
3. Enable the following APIs for your project:
   - Google Maps Platform APIs
   - Google Drive API
   - Google People API
   - Google Takeout API
4. Go to "APIs & Services" > "OAuth consent screen"
   - Set up the OAuth consent screen (External or Internal)
   - Add the following scope:
     - `https://www.googleapis.com/auth/dataportability.maps.starred_places`
5. Go to "APIs & Services" > "Credentials"
6. Create an OAuth client ID (Application type: Web application)
7. Add "http://localhost:4444/oauth2callback" as an authorized redirect URI
8. Note your Client ID and Client Secret for use with the CLI

## Environment Variables

You can set the following environment variables instead of using command-line options:

- `GOOGLE_MAPS_API_KEY`: Your Google Maps API key (for CSV conversion)
- `GOOGLE_OAUTH_CLIENT_ID`: Your Google OAuth client ID
- `GOOGLE_OAUTH_CLIENT_SECRET`: Your Google OAuth client secret

## Output Format

The tool outputs a JSON file with the following structure:

```json
{
  "locations": {
    "locationId1": {
      "id": "locationId1",
      "name": "Location Name",
      "address": "123 Main St, City, Country",
      "latitude": 37.7749,
      "longitude": -122.4194,
      "placeId": "googlePlaceId",
      "category": "favorite",
      "notes": "Location description",
      "createdAt": "2023-04-01T12:00:00Z",
      "updatedAt": "2023-04-01T12:00:00Z",
      "lists": ["Favorites", "Want to go"],
      "url": "https://maps.google.com/..."
    },
    // More locations...
  }
}
```

## Troubleshooting

### OAuth Errors

- **Error 400: invalid_scope**: Make sure you've added the required scope to your OAuth consent screen in the Google Cloud Console. The required scope is:
  - `https://www.googleapis.com/auth/dataportability.maps.starred_places`

- **Error 400: invalid_request**: If you see "Requests for data portability scopes cannot have non data portability scopes", make sure you're only requesting the data portability scope without mixing it with profile or email scopes.

- **Error: google.dataTransfer is not a function**: This error occurs when the Google Data Transfer API is not properly initialized. Make sure you're using the correct API version and that the Takeout API is enabled in your Google Cloud project.

- **Error: redirect_uri_mismatch**: Ensure that you've added `http://localhost:4444/oauth2callback` as an authorized redirect URI in your OAuth client settings.

- **Access denied by user**: The user declined to grant permission. Try again and accept all permission requests.

### API Errors

- **API not enabled**: Make sure you've enabled all the required APIs in your Google Cloud project:
  - Google Maps Platform APIs
  - Google Drive API
  - Google People API
  - Google Takeout API

- **Invalid API key**: Check that your Google Maps API key is correct and has the necessary permissions.

## License

MIT

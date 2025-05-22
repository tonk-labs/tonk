# Google Maps Locations Worker

A Tonk worker service that retrieves and syncs your saved Google Maps locations to your local-first Tonk environment.

## Overview

This worker connects to your Google account, exports your saved locations from Google Maps, and stores them in your Tonk environment. It runs on a daily schedule to keep your locations up-to-date.

## Prerequisites

- Google Cloud Platform account with OAuth 2.0 credentials
- Node.js 18 or higher
- Tonk CLI installed

## Getting Started

1. Install the worker:
   ```bash
   npm install -g @tonk/google-maps-locations-worker
   ```

2. Set up credentials and authentication:
   ```bash
   google-maps-locations-worker setup
   ```
   - This interactive setup will guide you through:
     1. Creating the required `credentials.json` file (you'll need to create OAuth 2.0 credentials in the Google Cloud Console)
     2. Running the OAuth authentication flow to create a `token.json` file
   - The setup will open a browser window for Google authentication
   - After authenticating, a `token.json` file will be created with refresh credentials
   - If you want to skip the authentication step, you can use the `--skip-auth` flag:
     ```bash
     google-maps-locations-worker setup --skip-auth
     ```

3. Create a `.env` file (optional):
   ```bash
   # Example .env file
   WORKER_PORT=5555
   KEEPSYNC_DOC_PATH=google-maps-locations
   ```

4. Start the worker:
   ```bash
   google-maps-locations-worker start
   ```
   - The worker will use the credentials and token created during setup
   - If you skipped authentication during setup, this will open a browser window for Google authentication

5. Register the worker with Tonk (if using Tonk):
   ```bash
   tonk worker register
   ```

## Development Setup

If you're developing or modifying the worker:

1. Clone the repository and install dependencies:
   ```bash
   git clone <repository-url>
   cd google-maps-locations-worker
   pnpm install
   ```

2. Set up credentials:
   ```bash
   pnpm dev
   ```

3. Build for production:
   ```bash
   pnpm build
   ```

## Deployment

After the initial authentication step, you can deploy the worker to run automatically:

1. Make sure both `credentials.json` and `token.json` files are included in your deployment
2. The worker will run without user intervention using the refresh token
3. Locations are exported daily at 3:00 AM by default

## API Endpoints

- `GET /health` - Health check endpoint
- `POST /export-locations` - Trigger a manual export of locations

## Worker Configuration

The worker is configured using the `worker.config.js` file. You can modify this file to change the worker's behavior.

## Project Structure

```
google-maps-locations-worker/
├── src/
│   ├── index.ts           # Main entry point
│   ├── cli.ts             # CLI for controlling the worker
│   ├── exportLocations.ts # Google Maps location export logic
│   └── utils.ts           # Utility functions
├── credentials.json       # Google OAuth credentials (you must create this)
├── token.json             # Authentication token (generated after first run)
├── worker.config.js       # Worker configuration
└── package.json           # Project configuration
```

## How It Works

1. The worker authenticates with Google using OAuth 2.0
2. It uses the Data Portability API to export your saved Google Maps locations
3. The exported data is downloaded as ZIP files containing CSV files
4. The CSV files are processed and converted to JSON
5. The locations are stored in your Tonk environment at the specified `KEEPSYNC_DOC_PATH`

## License

MIT © Tonk Labs

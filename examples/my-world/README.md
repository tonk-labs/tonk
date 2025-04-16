# My World App

Welcome to My World, a collaborative map application where you can save and share locations with friends.

## Features

- Save and manage locations on an interactive map
- Categorize locations with custom categories
- Add reviews to locations
- View business hours for locations (using Google Maps API)
- Compare locations with other users
- Offline-first with local storage and sync capabilities

## Setup

1. Install all dependencies (client and server):
```
npm run install:all
```

2. Set up environment variables:
Create a `.env` file with the following:
```
MAPKIT_TOKEN=your_apple_mapkit_token
GOOGLE_MAPS_API_KEY=your_google_maps_api_key
```

3. Start both the client and proxy server:
```
npm run dev:all
```

Alternatively, you can run them separately:
- Client: `npm run dev`
- Proxy Server: `npm run dev:server`

## API Proxy Server

For the Google Maps API integration to work properly, a proxy server is included to avoid CORS issues. The proxy server is located in the `server` directory and handles requests to the Google Maps API.

The proxy server runs on port 3001 by default and forwards requests to the Google Maps API while handling CORS issues.

## Building for Production

1. Build the client:
```
npm run build
```

2. Start the production server (which serves both the API and static files):
```
npm start
```

## New Features

### Business Hours
- The app now displays business hours for locations using the Google Maps API
- Open/closed status is shown in the location list and details
- Detailed business hours are displayed in the location details panel

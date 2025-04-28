# My World Proxy Server

This is a simple Express server that acts as a proxy for the Google Maps API requests from the My World application. It helps avoid CORS issues when making requests from the browser.

## Setup

1. Install dependencies:

```
cd server
npm install
```

2. Start the server:

```
npm start
```

For development with auto-restart:

```
npm run dev
```

## Environment Variables

The server uses the following environment variables from the parent project's `.env` file:

- `PORT`: The port on which the server will run (default: 3001)
- `GOOGLE_MAPS_API_KEY`: Your Google Maps API key

## API Endpoints

### GET /api/proxy

Proxies requests to the Google Maps API.

Query parameters:

- `url`: The URL to proxy (required)

Example:

```
GET /api/proxy?url=https://maps.googleapis.com/maps/api/place/details/json?place_id=PLACE_ID&fields=opening_hours&key=API_KEY
```

## Production

In production, the server also serves the static files from the React app's build directory. This allows you to deploy both the frontend and the proxy server together.

To build the React app for production:

```
cd ..
npm run build
```

Then start the server in production mode:

```
cd server
NODE_ENV=production npm start
```

# API Proxy Server for Development

This server provides API proxying for development mode. It reads the API service configurations from the frontend project and creates proxy endpoints for each service.

## Setup

1. Install dependencies:
   ```
   cd server
   pnpm install
   ```

2. Start the server:
   ```
   pnpm dev
   ```

## How it works

The server reads the API service configurations from `src/services/apiServices.ts` in the frontend project and creates proxy endpoints for each service. When a request is made to `/api/{service-prefix}/*`, the server proxies the request to the corresponding API endpoint, adding any required authentication.

In development mode, the Vite dev server is configured to proxy requests to `/api/*` to this server.

## Configuration

API services are configured in `src/services/apiServices.ts` in the frontend project. Each service has the following properties:

- `prefix`: The route prefix (e.g., "weather")
- `baseUrl`: The actual API base URL
- `requiresAuth`: Whether authentication is needed
- `authType`: Authentication type ("bearer", "apikey", "basic", or "query")
- `authHeaderName`: Header name for auth (e.g., "Authorization" or "X-API-Key")
- `authEnvVar`: API key or auth secret
- `authQueryParamName`: If using query auth type, the corresponding query param

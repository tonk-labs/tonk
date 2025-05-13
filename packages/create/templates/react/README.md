# Tonk App

Welcome to your new Tonk application!

## Usage

```bash
pnpm install
pnpm dev
```

This will start both the frontend development server and the API proxy server.

## API Proxy

The application includes an API proxy server that handles requests to external APIs. In development mode, the proxy server runs locally and forwards requests to the appropriate API endpoints, adding any required authentication.

API services are configured in `src/services/apiServices.ts`. Each service has the following properties:

- `prefix`: The route prefix (e.g., "weather")
- `baseUrl`: The actual API base URL
- `requiresAuth`: Whether authentication is needed
- `authType`: Authentication type ("bearer", "apikey", "basic", or "query")
- `authHeaderName`: Header name for auth (e.g., "Authorization" or "X-API-Key")
- `authEnvVar`: API key or auth secret
- `authQueryParamName`: If using query auth type, the corresponding query param

For more information about the API proxy server, see the [services README](./src/services/README.md) and the [server README](./server/README.md).

## Building for Production

```bash
pnpm build
```

This will build the frontend application and export the API configuration for production.

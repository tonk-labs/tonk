# Tonk App

Welcome to your new Tonk application!

## Development

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

## Deployment Options

### 1. Deploy to Tonk Server

You can deploy your app to a running Tonk server using the Tonk CLI:

```bash
tonk push
```

This will:
1. Build your application
2. Upload the bundle to your Tonk server

To start an uploaded Tonk app:

```bash
tonk start <bundleName>
```

### 2. Docker Deployment

This application comes with Docker support for easy self-hosting using the Tonk server.

#### Prerequisites

- Docker and Docker Compose installed on your system

#### Running with Docker Compose

```bash
# Build your application first
pnpm build

# Start the Tonk server container with your app
docker-compose up -d

# View logs
docker-compose logs -f

# Stop the container
docker-compose down
```

The Docker setup:
- Uses the official Tonk server container
- Automatically deploys your built application as a bundle
- Starts your application on port 8000

#### Environment Variables

Edit the `docker-compose.yml` file to add any required API keys or environment variables for your services.

#### Volumes

The Docker setup creates two persistent volumes:
- `tonk-data`: Stores the Tonk server's data
- `tonk-bundles`: Stores application bundles

#### Custom Docker Configuration

You can customize the Docker setup by editing the `docker-compose.yml` file.

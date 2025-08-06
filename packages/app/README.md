# Tonk App

Welcome to your new Tonk application!

## Development

```bash
pnpm install
pnpm dev
```

This will start both the frontend development server and the API proxy server.

## Development Server

The application includes a development server that runs alongside the frontend development server.

## Building for Production

```bash
pnpm build
```

This will build the frontend application for production.

## Deployment Options

### 1. Deploy to Tonk Server

You can deploy your app to a running Tonk server using the Tonk CLI:

```bash
tonk push
```

This will automatically:

1. Build your application with the correct base path
2. Upload the bundle to your Tonk server
3. Start the application on a route (e.g., `/file-browser`)

For more control, use:

```bash
# Skip building (if already built)
tonk push --no-build

# Upload only (don't start automatically)
tonk push --no-start

# Custom route
tonk push --route /my-custom-path
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

Edit the `docker-compose.yml` file to add any required API keys or environment variables for your
services.

#### Volumes

The Docker setup creates two persistent volumes:

- `tonk-data`: Stores the Tonk server's data
- `tonk-bundles`: Stores application bundles

#### Custom Docker Configuration

You can customize the Docker setup by editing the `docker-compose.yml` file.

# Tonk Server

Server package for Tonk applications with WebSocket sync capabilities

## Installation

```bash
npm install @tonk/server
```

## Features

- Express-based HTTP server for serving your Tonk application
- WebSocket server for real-time data synchronization
- Development and production modes
- Automatic WASM file handling and service worker integration for PWAs
- Docker template for production deployments

## Usage

### Basic Server Setup

```typescript
import { createServer } from '@tonk/server';

// Create and start a server in production mode
const server = await createServer({
  mode: 'production',
  distPath: './dist',
  port: 8080
});

// To stop the server
await server.stop();
```

### Development Mode

```typescript
import { createServer } from '@tonk/server';

// Create a development server for WebSocket sync
const server = await createServer({
  mode: 'development',
  distPath: undefined,
  port: 4080
});
```

### Server Options

The `createServer` function and `TonkServer` constructor accept the following options:

- `mode`: `'development'` or `'production'` (required)
- `distPath`: Path to the built frontend files (required for production mode)
- `port`: Server port (defaults to 4080 for development, 8080 for production)
- `verbose`: Enable/disable logging (defaults to true)

### Using the CLI Script

The package includes a standalone server script that can be used to serve your application:

```bash
# Add to your package.json scripts
"serve": "node node_modules/@tonk/server/scripts/serve.cjs"
```

This script will:
1. Check if your application is built, and build it if necessary
3. Create a local Express server
4. Serve your application with WebSocket sync capabilities

## WebSocket Sync Protocol

The server establishes a WebSocket endpoint at `/sync` that allows connected clients to synchronize data. Any message sent by a client is broadcast to all other connected clients.

## Development

1. Clone the repository
2. Install dependencies: `npm install`
3. Build the package: `npm run build`

## License

MIT © Tonk

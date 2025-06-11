# Tonk Workers

Tonk Workers are background services that extend your Tonk applications with additional functionality. They run as separate processes and integrate seamlessly with the Tonk ecosystem through a standardized API.

## Overview

Workers provide specialized functionality like:
- Data synchronisation with external services
- Scheduled background tasks
- API integrations
- Real-time data processing

## Architecture

Workers are standalone Node.js applications that:
- Run on their own ports
- Communicate via HTTP/WebSocket
- Integrate with Tonk's sync system (`keepsync`)
- Are managed by the Tonk CLI

## Creating a Worker

### 1. Initialize a Worker

```bash
tonk create # choose 'worker' when prompted
cd new-worker
```

### 2. Worker Structure

```typescript
// src/index.ts
import express from 'express';
import cors from 'cors';
import { configureSyncEngine } from './sync.js';

const app = express();
const PORT = process.env.PORT || 5555;

app.use(cors());
app.use(express.json());

// Health check endpoint (required)
app.get('/health', (req, res) => {
  res.json({ status: 'ok' });
});

// Main worker endpoint
app.post('/tonk', async (req, res) => {
  try {
    // Your worker logic here
    const result = await processRequest(req.body);
    res.json({ success: true, data: result });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

app.listen(PORT, () => {
  console.log(`Worker running on port ${PORT}`);
});
```

### 3. Worker Configuration

Create a `worker.config.js` file:

```javascript
module.exports = {
  runtime: {
    port: 5555,
    healthCheck: {
      endpoint: "/health",
      method: "GET",
      interval: 30000,
      timeout: 5000,
    },
  },
  process: {
    file: "index.js",
    cwd: "./dist",
    instances: 1,
    autorestart: true,
    env: {
      NODE_ENV: "production",
    },
  },
  schemas: {
    documents: {
      default: {},
    },
  },
};
```

## Managing Workers

### Register a Worker

```bash
tonk worker register
```

### List Workers

```bash
tonk worker ls
```

### Start/Stop Workers

```bash
tonk worker start my-worker
tonk worker stop my-worker
```

### Check Worker Status

```bash
tonk worker ping my-worker
tonk worker inspect my-worker
```

### View Logs

```bash
tonk worker logs my-worker
```

## `keepsync` Integration

Workers can read and write to Tonk's sync system:

```typescript
import { configureSyncEngine, readDoc, writeDoc } from './sync.js';

// Configure sync engine
const engine = configureSyncEngine({
  url: SYNC_URL,
  network: [wsAdapter as any as NetworkAdapterInterface],
  storage: new NodeFSStorageAdapter(),
});

// Read data
const data = await readDoc('my-collection/document-id');

// Write data
await writeDoc('my-collection/document-id', {
  timestamp: Date.now(),
  data: processedData,
});
```

## Example: Google Maps Locations Worker

The codebase includes a complete example worker that:
- Connects to Google Maps API
- Exports saved locations daily
- Stores data in `keepsync`
- Provides CLI commands for setup

Key features:
- OAuth 2.0 authentication
- Scheduled exports (cron-like)

## Standard Endpoints

Workers should implement these standard endpoints:

- `GET /health` - Health check (required)
- `POST /tonk` - Main processing endpoint
- `GET /status` - Worker status information
- Custom endpoints for specific functionality

## Best Practices

### Error Handling

```typescript
// Global error handlers
process.on('uncaughtException', (error) => {
  console.error('Uncaught Exception:', error);
  process.exit(1);
});

process.on('unhandledRejection', (reason) => {
  console.error('Unhandled Rejection:', reason);
  process.exit(1);
});

// Graceful shutdown
process.on('SIGINT', () => {
  console.log('Shutting down gracefully...');
  process.exit(0);
});
```

### Environment Configuration

```typescript
// Use environment variables for configuration
const config = {
  port: process.env.PORT || 5555,
  syncUrl: process.env.SYNC_URL || 'ws://localhost:7777',
  apiKey: process.env.API_KEY,
};
```

### Logging

```typescript
// Structured logging
console.log(JSON.stringify({
  timestamp: new Date().toISOString(),
  level: 'info',
  message: 'Worker started',
  port: PORT,
}));
```

## Deployment

Workers are deployed separately from Tonk applications but work great together.

1. **Development**: Workers run locally via `tonk worker start`
2. **Docker**: Include workers in your docker-compose.yml

## Troubleshooting

### Worker Not Starting

```bash
# Check worker status
tonk worker inspect my-worker

# View logs
tonk worker logs my-worker

# Check port conflicts
lsof -i :5555
```

### Health Check Failures

Ensure your worker responds to `GET /health` with:
```json
{"status": "ok"}
```

### Sync Issues

- Verify `SYNC_URL` environment variable
- Check network connectivity to Tonk server
- Ensure proper `keepsync` configuration

## Next Steps

- Learn about [Keepsync](./keepsync.md) for data synchronization
- Check out [deployment strategies](../deployment.md) for production

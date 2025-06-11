# Quickstart Guide

> If you haven't yet, start with the [**introduction**](./introduction.md) before reading this quickstart guide.

Tonk follows a simple two-mode workflow:

1. **Workers stream data in** - Background services connect to external APIs or your filesystem and stream data into your local Tonk store
2. **Apps visualise data** - Frontend applications provide beautiful interfaces to explore and interact with that data

This architecture separates data ingestion from visualisation, making your applications more maintainable and your data more reusable across different interfaces.

## Installing Tonk

First, you'll need to install Tonk on your machine:

```bash
npm install -g @tonk/cli && tonk hello
```

This will install the Tonk CLI globally and run the `hello` command, which sets up the Tonk daemon for synchronising your data.

## The Tonk Workflow

### Mode 1: Create Workers (Data Ingestion)

Workers are background services that connect to the outside world and stream data into your Tonk store. Start by creating a worker:

```bash
tonk create  # choose 'worker' when prompted
cd my-worker
```

Workers handle tasks like:
- Syncing data from Google Maps, Gmail, or other APIs
- Processing scheduled tasks
- Real-time data streaming
- API integrations

Example worker structure:
```typescript
// src/index.ts - Your worker's main logic
app.post('/tonk', async (req, res) => {
  // Connect to external API
  const data = await fetchFromExternalAPI();
  
  // Store in Tonk via keepsync
  await writeDoc('my-collection/data', data);
  
  res.json({ success: true });
});
```

### Mode 2: Create Apps (Data Visualisation)

Once you have data flowing in via workers, create frontend apps to visualise and interact with that data:

```bash
tonk create  # choose 'react' when prompted
cd my-app
```

Apps are React applications that:
- Connect to your Tonk stores
- Provide interfaces for your data
- Enable real-time collaboration
- Work offline-first

The CLI will scaffold a project with:
- React, TypeScript, and Tailwind CSS
- Keepsync for accessing your data stores
- Development tools and hot reloading

## Development Workflow

### Start Your Worker

First, get your worker running to begin data ingestion:

```bash
cd my-worker
pnpm dev
```

Register and start the worker:
```bash
tonk worker register
tonk worker start my-worker
```

### Start Your App

Then start your frontend app:

```bash
cd my-app
pnpm dev
```

This will:
1. Start a development server with hot reloading
2. Connect to your Tonk stores (where workers are streaming data)
3. Open your app in the browser (typically at http://localhost:3000)

## Understanding the Data Flow

The magic happens through **Tonk Stores** - shared data containers that connect workers and apps:

```
External APIs → Workers → Tonk Stores → Apps → Users
                  ↑                ↓
                  └ Real-time sync ┘
```

### In Workers: Writing Data

Workers stream data into stores using keepsync:

```typescript
import { writeDoc } from '@tonk/keepsync';

// Worker streams in location data
await writeDoc('locations/favorites', {
  places: googleMapsData,
  lastSync: Date.now()
});
```

### In Apps: Sculpting Data

Apps connect to these stores allowing you to perform sophisticated actions over your data.

```typescript
import { create } from "zustand";
import { sync } from "@tonk/keepsync";

// App reads and displays the same data
const useLocationStore = create(
  sync(
    (set) => ({
      locations: [],
      // ... your app logic
    }),
    { docId: "locations/favorites" }
  )
);
```

### Key Benefits

- **Separation of concerns**: Workers handle data, apps handle logic and rendering
- **Real-time sync**: Changes appear instantly across all connected apps
- **Offline-first**: Everything works without internet, syncs when reconnected
- **No database complexity**: No migrations, caching, or auth headaches
- **Collaborative**: Multiple users see updates in real-time

## Deployment Options

Tonk provides several ways to deploy your workers and apps:

### Local Deployment

Build and serve locally:

```bash
# Build your app
pnpm run build

# Push the bundle to Tonk
tonk push

# Start hosting the bundle
tonk start <bundleName>
```

### One-Touch Hosting (Experimental)

For quick deployment to the cloud:

```bash
tonk deploy
```

> ⚠️ **Note**: This is experimental and requires an access code. Contact Tonk support for access.

### Docker & Production

For production deployments, Tonk includes Docker support and cloud deployment options.

## Complete Example

Here's how a typical Tonk project structure looks:

```
my-tonk-project/
├── workers/
│   ├── gmail-sync/          # Worker: syncs Gmail data
│   ├── calendar-sync/       # Worker: syncs calendar events
│   └── api-processor/       # Worker: processes external APIs
└── apps/
    ├── dashboard/           # App: main dashboard view
    ├── mobile-viewer/       # App: mobile-friendly viewer
    └── admin-panel/         # App: admin interface
```

All workers stream data into shared Tonk stores, and all apps can access that data in real-time.

## Next Steps

- **Learn Workers**: [Tonk Workers Guide](./tonk-stack/workers.md) - Create background services
- **Learn Keepsync**: [Keepsync Guide](./tonk-stack/keepsync.md) - Master data synchronization  
- **Deploy to Production**: [Deployment Guide](./deployment.md) - Docker, cloud, and hosting options
- **CLI Reference**: [Command Reference](./reference.md) - Complete command documentation

## Real-World Examples

Check out these complete examples in the repository:
- **Google Maps Locations Worker** - Syncs your saved places from Google Maps
- **My World App** - Visualizes location data on an interactive map
- **File Browser App** - Browse and manage files with real-time sync

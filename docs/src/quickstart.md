# Quickstart Guide

Tonk follows a simple two-mode workflow:

1. **Workers stream data in** - Background services connect to external APIs or your file system and stream data into your local Tonk store
2. **Apps visualise data** - Frontend applications provide interfaces to explore and interact with that data

This architecture separates data ingestion from visualisation, making your applications more maintainable and your data more reusable across different interfaces.

## Installing Tonk

First, you'll need to install Tonk on your machine:

```bash
npm install -g @tonk/cli && tonk hello
```

This will install the Tonk CLI globally and run the `hello` command, which sets up the Tonk daemon for synchronising your data.

If you encounter issues at this stage, see the troubleshooting guide at the bottom of the page.

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
- Development tools

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

Then start your frontend app and navigate to [http://localhost:3000](http://localhost:3000) in the browser:

```bash
cd my-app
pnpm dev
```

This will:
1. Start a development server with hot reloading (so changes in the code are instantly reflected)
2. Connect to your Tonk stores (where workers are streaming data)

## Understanding the Data Flow

The magic happens through **Tonk Stores** - shared data containers that connect workers and apps:

```
External APIs → Workers → Tonk Stores → Apps → Users
                  ↑                ↓
                  └ Real-time sync ┘
```

### In Workers: Writing Data

Workers stream data into stores using `keepsync`:

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

Deploy in one command:

```bash
# Build, push, and start your app in one step
tonk push
```

Or for more control:

```bash
# Skip building (if already built)
tonk push --no-build

# Upload only (don't start automatically)
tonk push --no-start

# Then start manually later
tonk start <bundleName> --route /<route>
```

### One-Touch Hosting (Experimental)

For quick deployment to the cloud:

```bash
tonk deploy
```

> ⚠️ **Note**: This is experimental and requires an access code. Contact Tonk support for access.

### Docker & Production

For production deployments, Tonk includes Docker support and cloud deployment options.

## Next Steps

- **Learn Workers**: [Tonk Workers Guide](./workers.md) - Create background services
- **Learn Keepsync**: [Keepsync Guide](./keepsync.md) - Master data synchronization  
- **Deploy to Production**: [Deployment Guide](./deployment.md) - Docker, cloud, and hosting options
- **CLI Reference**: [Command Reference](./reference.md) - Complete command documentation

## Real-World Examples

Check out these complete examples in the repository:
- **Google Maps Locations Worker** - Syncs your saved places from Google Maps
- **My World App** - Visualizes location data on an interactive map
- **File Browser App** - Browse and manage files with real-time sync

## Troubleshooting
### Permission Denied Error During Installation

If you encounter a permission denied error when installing the Tonk CLI globally:

```bash
npm install -g @tonk/cli
# Error: EACCES: permission denied, mkdir '/usr/local/lib/node_modules'
```

This is a common npm issue on Unix systems. Here are several solutions:

#### Option 1: Fix npm permissions
```bash
sudo chown -R $(whoami) $(npm config get prefix)/{lib/node_modules,bin,share}
```

#### Option 2: Configure npm to use a different directory
```bash
mkdir ~/.npm-global
npm config set prefix '~/.npm-global'
echo 'export PATH="$HOME/.npm-global/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
npm install -g @tonk/cli
```

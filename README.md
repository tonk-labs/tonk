# Vibe code over your own data with Tonk

Tonk is a stack designed to simplify the process of building custom dashboards, tools, and AI agents
using your own data.

Tonk follows a simple two-mode workflow:

1. **Workers stream data in** - Background services connect to external APIs and stream data into
   your local Tonk store
2. **Apps visualise data** - Frontend applications provide interfaces to explore and interact with
   that data

This architecture separates data ingestion from visualisation, making applications more maintainable
and data reusable across different interfaces.

## Quick Start

```bash
# Install Tonk globally
npm install -g @tonk/cli && tonk hello

# Create a worker for data ingestion
tonk create  # choose 'worker'
cd my-worker && pnpm dev

# Create an app for visualisation
tonk create  # choose 'react'
cd my-app && pnpm dev
```

## Key Features

- **Local-First Architecture**: Conflict-free data synchronisation
- **Real-time Sync**: Changes appear instantly across all connected apps
- **Offline-First**: Everything works without internet, syncs when reconnected
- **Tailwind CSS**: Utility-first styling out of the box.
- **No Database Complexity**: No migrations, caching, or auth headaches
- **Package and Share**: Easily deploy and share your Tonk apps (work in progress).

You can use the Tonk toolchain to:

- Build applications within a copilot-friendly framework
- Manage complex state interoperable across people and apps
- Publish and share your apps

As an early stage project we are very open to feedback and keen to help builders - so please reach
out to the team and we will endeavour to support your usecase.

## Links

- [Docs](https://tonk-labs.github.io/tonk/quickstart.html)
- [Website](https://tonk.xyz)
- [GitHub](https://github.com/tonk-labs/tonk)
- [Community](https://t.me/+9W-4wDR9RcM2NWZk)

## License

Simplicity and freedom.

MIT © Tonk

<p align="center">
  <img src="https://github.com/user-attachments/assets/43586bd7-189e-4f4f-8196-ebe006beb115" />
</p>

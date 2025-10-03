# Quickstart Guide

> ⚠️ **Important**: Tonk is under heavy development and APIs are changing rapidly. Getting started
> requires manual setup and isn't for the faint of heart. We're working on making this easier!

## Prerequisites

Before you begin, you'll need to set up the development environment:

1. **Build core-js**:

```bash
cd packages/core-js
pnpm install
pnpm build
```

## Try the Example

The most complete example is `latergram`. Here's how to run it:

1. **Start the relay server** (required for sync):

```bash
cd packages/relay
pnpm dev
```

2. **Bundle the latergram example**:

```bash
cd examples/latergram
pnpm install
pnpm bundle create # Creates a .tonk file
touch .env #create .env file, see .env.example for required API_KEY, latergram uses anthropic claude
```

3. **Load it in host-web**:

```bash
cd packages/host-web
pnpm dev
# Then upload the .tonk file created in step 2
```

## Note on Templates

The `create` package has templates, but they're still in flux and may not work reliably. For now, we
recommend starting from the latergram example and modifying it to suit your needs.

## Examples in the Repository

Explore these working examples:

- **[latergram](/examples/latergram)** - Advanced application with dynamic components

## Next Steps

- [Architecture](./architecture.md) - Deep dive into Tonk's design
- [Virtual File System](./vfs.md) - Learn about the VFS layer
- [Bundle Format](./bundles.md) - Understand bundle packaging

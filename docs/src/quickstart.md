# Quickstart Guide

> ⚠️ **Important**: Tonk is under heavy development and APIs are changing rapidly. Getting started
> requires manual setup and isn't for the faint of heart. We're working on making this easier!

## Prerequisites

### Option 1: With Nix (Recommended)

The easiest way to get started is with Nix, which automatically sets up all dependencies:

```bash
# Install Nix with flakes support
curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install

# Enter development environment
cd tonk
nix develop

# Install dependencies
pnpm install
```

### Option 2: Manual Setup

If you prefer not to use Nix:

1. **Install dependencies**:
   - Node.js 20+
   - pnpm 9+
   - Rust toolchain (for building core)

2. **Set up relay binary**:

   You'll need access to the relay binary. Contact the Tonk team for details.

   ```bash
   export TONK_RELAY_BINARY=/path/to/tonk-relay
   ```

3. **Build core-js**:

   ```bash
   cd packages/core-js
   pnpm install
   pnpm build
   ```

## Try the Example

The most complete example is `latergram`. Here's how to run it:

> **Note**: With Nix, the relay server is automatically available via `$TONK_RELAY_BINARY`. Without
> Nix, ensure you've set the `TONK_RELAY_BINARY` environment variable.

1. **Bundle the latergram example**:

```bash
cd examples/latergram
pnpm install
pnpm bundle create # Creates a .tonk file
touch .env # Create .env file, see .env.example for required API_KEY (latergram uses Anthropic Claude)
```

2. **Load it in host-web**:

```bash
cd packages/host-web
pnpm dev
# Then upload the .tonk file created in step 1
```

## Note on Templates

The `create` package has templates, but they're still in flux and may not work reliably. For now, we
recommend starting from the latergram example and modifying it to suit your needs.

## Examples in the Repository

Explore these working examples:

- **[latergram](/examples/latergram)** - Advanced application with dynamic components

## Next Steps

- [Development Guide](../../DEVELOPMENT.md) - Complete setup guide including cross-repo development
- [Architecture](./architecture.md) - Deep dive into Tonk's design
- [Virtual File System](./vfs.md) - Learn about the VFS layer
- [Bundle Format](./bundles.md) - Understand bundle packaging

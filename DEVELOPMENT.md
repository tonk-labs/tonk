# Tonk Development Guide

This guide explains how to set up your development environment for working with Tonk.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Quick Start](#quick-start)
- [Without Nix](#without-nix)
- [Running Examples](#running-examples)
- [Building Packages](#building-packages)
- [Testing](#testing)
- [Code Quality](#code-quality)
- [Troubleshooting](#troubleshooting)

## Prerequisites

### With Nix (Recommended)

1. **Install Nix** with flakes support (see the [NixOS homepage](https://nixos.org/) and
   [Determinate Nix](https://docs.determinate.systems/) for details)

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install
   ```

2. **Install direnv** (optional but recommended):

   ```bash
   # macOS
   brew install direnv

   # Add to your shell rc file (~/.bashrc, ~/.zshrc, etc.)
   eval "$(direnv hook bash)"  # or zsh, fish, etc.
   ```

### Without Nix

1. **Node.js 20+** and **pnpm 9+**
2. **Rust toolchain**
3. **Docker**

## Quick Start

### With Nix + direnv

```bash
cd tonk
direnv allow  # Automatically loads the environment
pnpm install
```

### With Nix (manual)

```bash
cd tonk
nix develop  # Enters development shell
pnpm install
```

### Without Nix

If you prefer not to use Nix, you can set everything up manually:

```bash
# Node.js, pnpm
brew install node pnpm  # macOS
# or use nvm, fnm, etc.

# Rust (for building relay)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install dependencies
pnpm install
```

## Repository Structure

- `packages/core` - CRDT core (Rust)
- `packages/core-js` - TypeScript bindings
- `packages/host-web` - Web host environment
- `packages/relay` - Sync relay (Rust)
- `packages/launcher` - Tonk Launcher
- `packages/desktonk` - Bundle development environment
- `tests/` - Integration tests
- `examples/` - Example applications

### Using an Alternate Relay

By default, the relay in `packages/relay` is used. To use a different relay binary:

```bash
export TONK_RELAY_BINARY=/path/to/alternate/tonk-relay
```

All tools, tests, and examples will use the specified binary.

## Running Examples

### Demo Application

```bash
cd examples/demo
pnpm run dev
```

Open http://localhost:4000

### Other Examples

Each example follows the same pattern:

```bash
cd examples/<example-name>
pnpm install
pnpm run dev
```

## Code Quality

### Linting and Formatting

The project uses ESLint for linting and Prettier for code formatting.

#### Available Scripts

**Root level** (runs across all packages):

```bash
# Check linting issues
pnpm run lint

# Auto-fix linting issues
pnpm run lint:fix

# Format code
pnpm run format

# Check formatting
pnpm run format:check

# Run both linting and formatting checks
pnpm run lint:all

# Auto-fix both linting and formatting
pnpm run fix:all
```

### Pre-commit Hooks

The repository uses Husky for pre-commit hooks that automatically:

- Run ESLint with auto-fix
- Format code with Prettier
- Stage the fixed files

## Building Packages

```bash
# Build all packages
pnpm run build

# Build specific package
cd packages/host-web && pnpm run build
```

## Testing

```bash
pnpm test
```

## Troubleshooting

### "Relay binary not found" Error

The relay should build automatically, but if you encounter issues:

```bash
# Build the relay manually
cd packages/relay
cargo build --release

# Verify it exists
ls -la target/release/tonk-relay
```

### Nix Flake Update Issues

```bash
# Update flake inputs
nix flake update

# Clear flake cache
rm -rf ~/.cache/nix

# Force rebuild
nix develop --refresh
```

### direnv Not Loading

```bash
# Allow direnv
direnv allow

# Check status
direnv status

# Reload
direnv reload
```

### Port Already in Use

```bash
# Find process using port 8081
lsof -i :8081

# Kill it
kill -9 <PID>

# Or use a different port
$TONK_RELAY_BINARY 8082 app.tonk
```

### Linting Issues

If you encounter linting errors:

1. Run `pnpm run lint:fix` to auto-fix issues
2. For remaining issues, fix them manually
3. Use ESLint disable comments sparingly

### Build Issues

If builds fail:

1. Ensure all dependencies are installed: `pnpm install`
2. Clean and rebuild: `pnpm run clean && pnpm run build`

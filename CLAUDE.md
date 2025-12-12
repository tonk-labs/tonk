# Tonk Monorepo - Claude Code Guidelines

## Package Manager - CRITICAL

**USE BUN ONLY. DO NOT USE pnpm, npm, OR yarn.**

This repository has been migrated to Bun. All package management, script execution, and testing must use Bun:

```bash
# CORRECT
bun install
bun run build
bun run dev
bun test

# WRONG - DO NOT USE
pnpm install  # NO
npm install   # NO
yarn install  # NO
```

### Exception: Playwright Tests

Playwright tests must use `npx` due to compatibility requirements:
```bash
npx playwright test
```

## Repository Structure

- `packages/core` - CRDT core (Rust)
- `packages/core-js` - TypeScript bindings
- `packages/host-web` - Web host environment
- `packages/relay` - Sync relay (Rust)
- `packages/launcher` - Tonk Launcher
- `packages/desktonk` - Bundle development environment
- `tests/` - Integration tests (Playwright - uses npx)
- `examples/` - Example applications

## Development Commands

```bash
# Install dependencies
bun install

# Build all packages
bun run build

# Run linting
bun run lint
bun run lint:fix

# Format code
bun run format

# Run tests
bun test
```

## Code Style

- Use Biome for linting and formatting (not Prettier/ESLint)
- Prefix unused parameters with `_` (e.g., `_req`)
- Follow existing patterns in the codebase

## Keepsync Integration

For synchronization features, read the keepsync documentation:
- `docs/src/llms/shared/keepsync/` - Core keepsync docs
- `docs/src/llms/shared/keepsync/examples/` - Usage examples

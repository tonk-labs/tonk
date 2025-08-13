# Development Guide

This guide covers the development setup and workflow for the Tonk monorepo.

## Setup

1. **Install dependencies**:

   ```bash
   pnpm install
   ```

2. **Install recommended VS Code extensions**:
   - ESLint
   - Prettier
   - TypeScript

## Code Quality

### Linting and Formatting

The project uses ESLint for linting and Prettier for code formatting with consistent configuration
across all packages.

#### Available Scripts

**Root level** (runs across all packages):

```bash
# Check linting issues
npm run lint

# Auto-fix linting issues
npm run lint:fix

# Format code
npm run format

# Check formatting
npm run format:check

# Run both linting and formatting checks
npm run lint:all

# Auto-fix both linting and formatting
npm run fix:all
```

**Package level** (run from within any package directory):

```bash
# Check linting issues
pnpm run lint

# Auto-fix linting issues
pnpm run lint:fix

# Format code
pnpm run format

# Check formatting
pnpm run format:check
```

### Pre-commit Hooks

The repository is configured with pre-commit hooks that automatically:

- Run ESLint with auto-fix
- Format code with Prettier
- Stage the fixed files

This ensures all committed code follows the project's style guidelines.

### VS Code Integration

The workspace is configured to:

- Format code on save
- Auto-fix ESLint issues on save
- Use Prettier as the default formatter
- Validate TypeScript files with ESLint

## Package Structure

The monorepo contains the following packages:

- **`@tonk/cli`** - Command line interface
- **`@tonk/create`** - Project scaffolding tool
- **`@tonk/keepsync`** - Sync engine framework
- **`@tonk/server`** - Server package

Each package has its own:

- `package.json` with build, test, and lint scripts
- TypeScript configuration
- Individual dependencies

## Building Packages

```bash
# Build all packages
pnpm run build

# Build specific package
cd packages/cli && pnpm run build
```

## Testing

There are two scripts - `setup-production.js` and `setup-staging.js` - that will configure the
`tonk` and `knot` monorepo environments for end-to-end testing against staging and production.

IMPORTANT: Though the scripts exist in `tonk`, they expect both `tonk` and `knot` to exist in the
same directory. The scripts will not work without this condition met.

```bash
# Run tests for all packages
pnpm run test

# Run tests for specific package
cd packages/cli && pnpm run test
```

## Publishing

See [PUBLISHING.md](./PUBLISHING.md) for details on the automated publishing workflow.

## Code Style Guidelines

### TypeScript

- Use explicit types where helpful for readability
- Prefer `const` over `let` where possible
- Use meaningful variable and function names
- Add JSDoc comments for public APIs

### Imports

- Use relative imports within packages
- Group imports: external libraries first, then internal modules
- Sort imports alphabetically within groups

### Error Handling

- Use proper error types
- Provide meaningful error messages
- Handle async operations with proper error catching

## Troubleshooting

### Linting Issues

If you encounter linting errors:

1. Run `npm run lint:fix` to auto-fix issues
2. For remaining issues, fix them manually
3. If you need to disable a rule, use ESLint disable comments sparingly

### Build Issues

If builds fail:

1. Ensure all dependencies are installed: `pnpm install`
2. Clean and rebuild: `pnpm run clean && pnpm run build`
3. Check for TypeScript errors: `pnpm run type-check`

### Pre-commit Hook Issues

If the pre-commit hook fails:

1. The hook will show which files have issues
2. Fix the issues manually or run `npm run fix:all`
3. Stage the changes and commit again

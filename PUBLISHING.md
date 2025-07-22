# NPM Package Publishing

This repository is set up with automated NPM package publishing when package versions are bumped.

## How it works

1. **Version Detection**: The GitHub workflow monitors changes to `packages/*/package.json` files
2. **Automatic Publishing**: When a version number changes in any package.json, the workflow
   automatically:
   - Builds the package
   - Runs tests and linting (if available)
   - Publishes to NPM (if the version doesn't already exist)
   - Creates a GitHub release

## Publishing a new version

### Option 1: Using the helper script (Recommended)

```bash
# Bump patch version (0.3.2 → 0.3.3)
node scripts/bump-version.js cli patch

# Bump minor version (0.3.2 → 0.4.0)
node scripts/bump-version.js keepsync minor

# Bump major version (0.3.2 → 1.0.0)
node scripts/bump-version.js server major
```

Then commit and push:

```bash
git commit -m "bump: cli v0.3.3"
git push origin main
```

### Option 2: Manual version bump

1. Edit the `version` field in the package's `package.json`
2. Commit and push the change to the `main` branch
3. The workflow will automatically detect the version change and publish

## Available packages

- `@tonk/cli` - The Tonk stack command line utility
- `@tonk/create` - Bootstrap apps on the Tonk stack
- `@tonk/keepsync` - A reactive sync engine framework
- `@tonk/server` - Server package for Tonk applications

## Prerequisites

### NPM Token Setup

To enable publishing, you need to set up an NPM token:

1. Create an NPM account and generate an access token at https://www.npmjs.com/settings/tokens
2. Add the token as a repository secret named `NPM_TOKEN`:
   - Go to your repository settings
   - Navigate to Secrets and variables → Actions
   - Add a new repository secret with name `NPM_TOKEN` and your token as the value

### Package Configuration

All packages are configured with:

- `"access": "public"` in publishConfig (for scoped packages)
- Proper build scripts that run before publishing
- Repository and homepage URLs pointing to this monorepo

## Workflow Features

- **Smart Detection**: Only publishes packages whose versions have actually changed
- **Duplicate Prevention**: Checks if a version already exists on NPM before attempting to publish
- **Build Validation**: Runs build, test, and lint scripts before publishing
- **GitHub Releases**: Automatically creates GitHub releases for published packages
- **Parallel Publishing**: Multiple packages can be published simultaneously if multiple versions
  are bumped

## Troubleshooting

### Package already exists error

The workflow checks if a version already exists before publishing. If you see this message, the
version has already been published to NPM.

### Build failures

If the build, test, or lint steps fail, the package won't be published. Check the workflow logs for
details.

### Permission errors

Ensure the `NPM_TOKEN` secret is properly configured and has publish permissions for the `@tonk`
scope.

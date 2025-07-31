#!/usr/bin/env node

import { readFileSync, writeFileSync } from 'fs';
import { join } from 'path';
import { execSync } from 'child_process';

const PACKAGES_DIR = 'packages';
const VALID_BUMP_TYPES = ['patch', 'minor', 'major'];

function usage() {
  console.log(`
Usage: node scripts/bump-version.js <package-name> <bump-type>

Arguments:
  package-name  Name of the package to bump (cli, create, keepsync, server)
  bump-type     Type of version bump (patch, minor, major)

Examples:
  node scripts/bump-version.js cli patch
  node scripts/bump-version.js keepsync minor
  node scripts/bump-version.js server major
`);
  process.exit(1);
}

function bumpVersion(version, bumpType) {
  const [major, minor, patch] = version.split('.').map(Number);

  switch (bumpType) {
    case 'major':
      return `${major + 1}.0.0`;
    case 'minor':
      return `${major}.${minor + 1}.0`;
    case 'patch':
      return `${major}.${minor}.${patch + 1}`;
    default:
      throw new Error(`Invalid bump type: ${bumpType}`);
  }
}

function main() {
  const [, , packageName, bumpType] = process.argv;

  if (!packageName || !bumpType) {
    usage();
  }

  if (!VALID_BUMP_TYPES.includes(bumpType)) {
    console.error(
      `Error: Invalid bump type "${bumpType}". Must be one of: ${VALID_BUMP_TYPES.join(', ')}`
    );
    process.exit(1);
  }

  const packagePath = join(PACKAGES_DIR, packageName);
  const packageJsonPath = join(packagePath, 'package.json');

  try {
    // Read current package.json
    const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
    const currentVersion = packageJson.version;
    const newVersion = bumpVersion(currentVersion, bumpType);

    console.log(
      `Bumping ${packageName} from ${currentVersion} to ${newVersion}`
    );

    // Update version
    packageJson.version = newVersion;

    // Write updated package.json
    writeFileSync(packageJsonPath, JSON.stringify(packageJson, null, 2) + '\n');

    // Stage the change
    execSync(`git add ${packageJsonPath}`, { stdio: 'inherit' });

    console.log(`✅ Successfully bumped ${packageName} to ${newVersion}`);
    console.log(
      `📝 Changes staged. Commit and push to trigger auto-publishing.`
    );
  } catch (error) {
    if (error.code === 'ENOENT') {
      console.error(
        `Error: Package "${packageName}" not found in ${PACKAGES_DIR}/`
      );
      console.error(`Available packages: cli, create, keepsync, server`);
    } else {
      console.error(`Error: ${error.message}`);
    }
    process.exit(1);
  }
}

main();

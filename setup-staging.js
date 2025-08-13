#!/usr/bin/env node

/**
 * Setup script to configure Tonk for staging environment
 * This script coordinates building tonk-auth with staging config and linking it to the CLI
 */

import { execSync } from 'child_process';
import { existsSync } from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const KNOT_PATH = path.join(__dirname, '..', 'knot');
const TONK_PATH = __dirname;
const TONK_AUTH_PATH = path.join(KNOT_PATH, 'tonk-auth');
const TONK_CLI_PATH = path.join(TONK_PATH, 'packages', 'cli');

function run(command, cwd = process.cwd()) {
  console.log(`🔧 Running: ${command}`);
  console.log(`📁 In: ${cwd}`);

  try {
    execSync(command, {
      cwd,
      stdio: 'inherit',
      env: { ...process.env, TONK_ENV: 'staging' },
    });
  } catch (error) {
    console.error(`❌ Command failed: ${command}`);
    process.exit(1);
  }
}

function checkPath(pathToCheck, name) {
  if (!existsSync(pathToCheck)) {
    console.error(`❌ ${name} not found at: ${pathToCheck}`);
    process.exit(1);
  }
  console.log(`✅ Found ${name} at: ${pathToCheck}`);
}

console.log('🚀 Setting up Tonk for staging environment...\n');

// Verify paths exist
checkPath(KNOT_PATH, 'knot directory');
checkPath(TONK_PATH, 'tonk directory');
checkPath(TONK_AUTH_PATH, 'tonk-auth');
checkPath(TONK_CLI_PATH, 'tonk CLI');

console.log('\n📦 Building tonk-auth with staging configuration...');
run('pnpm build:staging', TONK_AUTH_PATH);

console.log('\n🔗 Linking tonk-auth to CLI...');
run(`pnpm link ${TONK_AUTH_PATH}`, TONK_CLI_PATH);

console.log('\n🏗️ Building tonk CLI...');
run('TONK_ENV=staging pnpm run build', TONK_CLI_PATH);

console.log('\n✅ Staging setup complete!');
console.log('\n📋 To use staging environment:');
console.log('   export TONK_ENV=staging');
console.log('   # or run commands with: TONK_ENV=staging tonk <command>');
console.log('\n📋 To switch back to production:');
console.log('   unset TONK_ENV');
console.log('   # or run: node setup-production.js');


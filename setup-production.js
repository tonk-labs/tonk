#!/usr/bin/env node

/**
 * Setup script to configure Tonk for production environment
 * This script unlinks staging tonk-auth and reinstalls the published version
 */

import { execSync } from 'child_process';
import { existsSync } from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const TONK_PATH = __dirname;
const TONK_CLI_PATH = path.join(TONK_PATH, 'packages', 'cli');

function run(command, cwd = process.cwd()) {
  console.log(`🔧 Running: ${command}`);
  console.log(`📁 In: ${cwd}`);

  try {
    execSync(command, {
      cwd,
      stdio: 'inherit',
      env: { ...process.env, TONK_ENV: 'production' },
    });
  } catch (error) {
    console.error(`❌ Command failed: ${command}`);
    // Don't exit on unlink failures - they might not be linked
    if (!command.includes('unlink')) {
      process.exit(1);
    }
  }
}

function checkPath(pathToCheck, name) {
  if (!existsSync(pathToCheck)) {
    console.error(`❌ ${name} not found at: ${pathToCheck}`);
    process.exit(1);
  }
  console.log(`✅ Found ${name} at: ${pathToCheck}`);
}

console.log('🚀 Setting up Tonk for production environment...\n');

// Verify paths exist
checkPath(TONK_PATH, 'tonk directory');
checkPath(TONK_CLI_PATH, 'tonk CLI');

console.log('\n🔗 Unlinking local tonk-auth...');
run('pnpm unlink @tonk/tonk-auth', TONK_CLI_PATH);

console.log('\n📦 Reinstalling published tonk-auth...');
run('pnpm install @tonk/tonk-auth@latest', TONK_CLI_PATH);

console.log('\n🏗️ Building tonk CLI...');
run('TONK_ENV=production pnpm run build', TONK_CLI_PATH);

console.log('\n✅ Production setup complete!');
console.log('\n📋 Now using published @tonk/tonk-auth package');
console.log('📋 Environment is set to production by default');


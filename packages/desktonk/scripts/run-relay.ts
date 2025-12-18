#!/usr/bin/env bun

/**
 * Relay runner script - uses @tonk/relay npm package programmatic API
 *
 * Platform support: darwin-arm64 only (Apple Silicon)
 * Other platforms: Use relay.tonk.xyz production server
 */

import { existsSync } from 'node:fs';

const PORT = Number(process.env.PORT) || 8081;
const BUNDLE_FILE = 'app.tonk';

async function main() {
  console.log('Starting Tonk relay server...\n');

  // Check platform
  if (process.platform !== 'darwin' || process.arch !== 'arm64') {
    console.warn(
      'WARNING: @tonk/relay currently only supports darwin-arm64 (Apple Silicon)'
    );
    console.warn(`Your platform: ${process.platform}-${process.arch}`);
    console.warn('');
    console.warn('Options:');
    console.warn(
      '  1. Use relay.tonk.xyz for development (set TONK_SERVER_URL)'
    );
    console.warn('  2. Run on an Apple Silicon Mac');
    console.warn('  3. Wait for more platform binaries to be published');
    console.warn('');
    console.warn('Attempting to start anyway...\n');
  }

  // Check if bundle exists
  if (!existsSync(BUNDLE_FILE)) {
    console.log(`No ${BUNDLE_FILE} found.`);
    console.log('Build one with: bun run bundle\n');
    console.log('Waiting for bundle to appear...');

    // Poll for bundle existence
    await new Promise<void>(resolve => {
      const interval = setInterval(() => {
        if (existsSync(BUNDLE_FILE)) {
          console.log('\nBundle detected! Starting relay...\n');
          clearInterval(interval);
          resolve();
        }
      }, 2000);
    });
  }

  // Import and start relay using programmatic API
  try {
    const { startRelay } = await import('@tonk/relay');

    console.log(`Relay starting on port ${PORT}...`);
    console.log(`Using bundle: ${BUNDLE_FILE}\n`);

    const relay = startRelay({
      port: PORT,
      bundlePath: `./${BUNDLE_FILE}`,
    });

    relay.on('exit', (code: number) => {
      console.log(`\nRelay exited with code ${code}`);
      process.exit(code);
    });

    // Handle termination signals
    process.on('SIGINT', () => {
      console.log('\nStopping relay...');
      relay.kill();
    });

    process.on('SIGTERM', () => {
      relay.kill();
    });
  } catch (err) {
    console.error('Failed to start relay:', err);
    console.error('\nMake sure @tonk/relay is installed:');
    console.error('  bun install\n');

    if (process.platform !== 'darwin' || process.arch !== 'arm64') {
      console.error('NOTE: @tonk/relay currently only supports darwin-arm64');
    }

    process.exit(1);
  }
}

main();

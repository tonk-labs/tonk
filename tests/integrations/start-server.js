#!/usr/bin/env node

import { createServer } from '@tonk/server';
import path from 'path';
import fs from 'fs';
import os from 'os';

// Default port
const PORT = process.env.PORT ? parseInt(process.env.PORT, 10) : 8888;

// Create temp directories for testing
const tempDir = path.join(os.tmpdir(), 'tonk-integration-test');
const bundlesPath = path.join(tempDir, 'bundles');
const storesPath = path.join(tempDir, 'stores');
const configPath = path.join(tempDir, 'config.json');

// Ensure the directories exist
if (!fs.existsSync(tempDir)) {
  fs.mkdirSync(tempDir, { recursive: true });
}
if (!fs.existsSync(bundlesPath)) {
  fs.mkdirSync(bundlesPath, { recursive: true });
}
if (!fs.existsSync(storesPath)) {
  fs.mkdirSync(storesPath, { recursive: true });
}

console.log('Starting Tonk server for integration tests...');
console.log(`PORT: ${PORT}`);
console.log(`Data directory: ${tempDir}`);
console.log(`Config path: ${configPath}`);

// Create and start the server
const startServer = async () => {
  try {
    const server = await createServer({
      port: PORT,
      bundlesPath,
      dirPath: storesPath,
      verbose: true,
      configPath,
    });

    console.log('Tonk integration test server is running!');
    console.log(`Server is available at http://localhost:${PORT}`);
    console.log('Press Ctrl+C to stop');

    // Handle process termination
    const cleanup = async () => {
      console.log('\nShutting down server...');
      try {
        await server.stop();
        console.log('Server shutdown complete.');
        setTimeout(() => process.exit(0), 500);
      } catch (err) {
        console.error('Error during shutdown:', err);
        process.exit(1);
      }
    };

    process.on('SIGINT', cleanup);
    process.on('SIGTERM', cleanup);
  } catch (error) {
    console.error('Failed to start Tonk integration test server:', error);
    process.exit(1);
  }
};

startServer(); 
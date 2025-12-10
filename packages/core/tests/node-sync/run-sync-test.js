#!/usr/bin/env node

import { spawn } from 'child_process';
import { testBasicRelay } from './test-client.js';
import { testSamodDirectSync } from './test-samod-direct.js';

// Test runner that manages server lifecycle and runs tests
class TestRunner {
  constructor() {
    this.serverProcess = null;
  }

  async startServer() {
    return new Promise((resolve, reject) => {
      console.log('Starting WebSocket relay server...');

      this.serverProcess = spawn('node', ['test-server.js'], {
        stdio: ['pipe', 'pipe', 'pipe'],
        cwd: process.cwd(),
      });

      let serverReady = false;

      this.serverProcess.stdout.on('data', data => {
        const output = data.toString();
        console.log(`[SERVER] ${output.trim()}`);

        if (output.includes('Ready to relay messages')) {
          serverReady = true;
          resolve();
        }
      });

      this.serverProcess.stderr.on('data', data => {
        console.error(`[SERVER ERROR] ${data.toString().trim()}`);
      });

      this.serverProcess.on('close', code => {
        console.log(`[SERVER] Process exited with code ${code}`);
        this.serverProcess = null;
      });

      this.serverProcess.on('error', error => {
        console.error(`[SERVER] Failed to start:`, error);
        if (!serverReady) {
          reject(error);
        }
      });

      // Timeout if server doesn't start within 5 seconds
      setTimeout(() => {
        if (!serverReady) {
          reject(new Error('Server failed to start within timeout'));
        }
      }, 5000);
    });
  }

  stopServer() {
    if (this.serverProcess) {
      console.log('Stopping WebSocket relay server...');
      this.serverProcess.kill('SIGTERM');
      this.serverProcess = null;
    }
  }

  async runTests() {
    console.log('🚀 Starting samod sync tests\n');

    try {
      // Start the relay server
      await this.startServer();

      // Wait a moment for server to be fully ready
      await new Promise(resolve => setTimeout(resolve, 500));

      console.log('Server started successfully, running tests...\n');

      // Run basic relay test
      await testBasicRelay();

      // Run direct samod sync test
      await testSamodDirectSync();

      console.log('=== All Tests Complete ===');
      return true;
    } catch (error) {
      console.error('❌ Test suite failed:', error);
      return false;
    } finally {
      this.stopServer();
    }
  }
}

// Run tests if this file is executed directly
if (process.argv[1] === new URL(import.meta.url).pathname) {
  const runner = new TestRunner();

  runner
    .runTests()
    .then(success => {
      process.exit(success ? 0 : 1);
    })
    .catch(error => {
      console.error('Test runner failed:', error);
      process.exit(1);
    });

  // Handle cleanup on exit
  process.on('SIGINT', () => {
    console.log('\n🛑 Received SIGINT, cleaning up...');
    runner.stopServer();
    process.exit(0);
  });

  process.on('SIGTERM', () => {
    console.log('\n🛑 Received SIGTERM, cleaning up...');
    runner.stopServer();
    process.exit(0);
  });
}

export { TestRunner };

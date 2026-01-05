#!/usr/bin/env node

/**
 * Test runner for TonkCore sync tests
 * Manages server lifecycle and runs tests
 */

// Polyfill WebSocket for Node.js
const WebSocket = require("ws");
global.WebSocket = WebSocket;

const { spawn } = require("child_process");
const path = require("path");
const { testTonkDirectSync } = require("./test-sync-direct.js");

// Path to the proper automerge-repo sync server
const SYNC_SERVER_DIR = path.resolve(__dirname, "../../examples/server");

// Test runner that manages server lifecycle and runs tests
class TestRunner {
  constructor() {
    this.serverProcess = null;
  }

  async startSyncServer() {
    return new Promise((resolve, reject) => {
      console.log("Starting automerge-repo sync server...");
      console.log(`Server directory: ${SYNC_SERVER_DIR}`);

      // Use npx tsx to run the TypeScript server
      this.serverProcess = spawn("npx", ["tsx", "server.ts", "8081"], {
        stdio: ["pipe", "pipe", "pipe"],
        cwd: SYNC_SERVER_DIR,
        shell: true,
      });

      let serverReady = false;

      this.serverProcess.stdout.on("data", (data) => {
        const output = data.toString();
        console.log(`[SYNC-SERVER] ${output.trim()}`);

        if (output.includes("Listening on port")) {
          serverReady = true;
          resolve();
        }
      });

      this.serverProcess.stderr.on("data", (data) => {
        const output = data.toString().trim();
        // Filter out npm/npx noise
        if (!output.includes("npm") && output.length > 0) {
          console.error(`[SYNC-SERVER STDERR] ${output}`);
        }
      });

      this.serverProcess.on("close", (code) => {
        console.log(`[SYNC-SERVER] Process exited with code ${code}`);
        this.serverProcess = null;
      });

      this.serverProcess.on("error", (error) => {
        console.error(`[SYNC-SERVER] Failed to start:`, error);
        if (!serverReady) {
          reject(error);
        }
      });

      // Timeout if server doesn't start within 15 seconds (tsx may take a moment)
      setTimeout(() => {
        if (!serverReady) {
          reject(new Error("Sync server failed to start within timeout"));
        }
      }, 60000);
    });
  }

  stopSyncServer() {
    if (this.serverProcess) {
      console.log("Stopping sync server...");
      this.serverProcess.kill("SIGTERM");
      this.serverProcess = null;
    }
  }

  async runTests() {
    console.log("Starting TonkCore sync tests\n");

    try {
      // Start the automerge-repo sync server for all tests
      console.log("Starting sync server...");
      await this.startSyncServer();
      await new Promise((resolve) => setTimeout(resolve, 1000));
      console.log("Sync server started\n");

      // Run TonkCore VFS document sync test
      console.log("Running TonkCore sync test...\n");
      await testTonkDirectSync();

      console.log("=== All Tests Complete ===");
      return true;
    } catch (error) {
      console.error("*** Test suite failed:", error);
      return false;
    } finally {
      this.stopSyncServer();
    }
  }
}

// Run tests if this file is executed directly
if (require.main === module) {
  const runner = new TestRunner();

  runner
    .runTests()
    .then((success) => {
      process.exit(success ? 0 : 1);
    })
    .catch((error) => {
      console.error("Test runner failed:", error);
      process.exit(1);
    });

  // Handle cleanup on exit
  process.on("SIGINT", () => {
    console.log("\nReceived SIGINT, cleaning up...");
    runner.stopSyncServer();
    process.exit(0);
  });

  process.on("SIGTERM", () => {
    console.log("\nReceived SIGTERM, cleaning up...");
    runner.stopSyncServer();
    process.exit(0);
  });
}

module.exports = { TestRunner };

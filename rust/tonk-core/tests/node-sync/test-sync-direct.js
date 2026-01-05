#!/usr/bin/env node

/**
 * Direct sync test for TonkCore
 * Tests document creation, retrieval, and sync between peers
 */

const fs = require("fs");
const path = require("path");

// Path to shared bundle - both clients load from this to share the same root
const BUNDLE_PATH = path.resolve(__dirname, "../data/blank.tonk");

// Load the WASM module
let wasm;
try {
  wasm = require(path.resolve(__dirname, "../../pkg-node/tonk_core.js"));
} catch (error) {
  console.error("Failed to load WASM module. Make sure to build it first.");
  console.error("Error:", error.message);
  process.exit(1);
}

class TonkTestClient {
  constructor(clientId, serverUrl = "ws://127.0.0.1:8081") {
    this.clientId = clientId;
    this.serverUrl = serverUrl;
    this.tonk = null;
    this.connected = false;
  }

  /**
   * Initialize the TonkCore instance
   * @param {Buffer|Uint8Array} bundleBytes - Optional bundle bytes to create from shared root
   */
  async init(bundleBytes = null) {
    try {
      console.log(`[${this.clientId}] Creating TonkCore instance...`);

      if (bundleBytes) {
        // Create from shared bundle so both instances have the same root document
        console.log(
          `[${this.clientId}] Loading from shared bundle (${bundleBytes.length} bytes)`,
        );
        this.tonk = await wasm.create_tonk_from_bytes(
          new Uint8Array(bundleBytes),
        );
      } else {
        // Create fresh instance (for local-only tests)
        this.tonk = await wasm.create_tonk();
      }

      console.log(`[${this.clientId}] TonkCore created successfully`);
      const peerId = await this.tonk.getPeerId();
      console.log(`[${this.clientId}] Peer ID: ${peerId}`);

      return true;
    } catch (error) {
      console.error(`[${this.clientId}] Failed to create TonkCore:`, error);
      return false;
    }
  }

  async connect() {
    if (!this.tonk) {
      throw new Error(`[${this.clientId}] Must call init() first`);
    }

    try {
      console.log(
        `[${this.clientId}] Connecting to WebSocket: ${this.serverUrl}`,
      );
      await this.tonk.connectWebsocket(this.serverUrl);
      console.log(`[${this.clientId}] WebSocket connection initiated`);

      // Wait for connection to reach "connected" state (fully synced)
      const maxWaitMs = 5000;
      const pollIntervalMs = 100;
      let waited = 0;

      while (waited < maxWaitMs) {
        const state = await this.tonk.getConnectionState();
        console.log(`[${this.clientId}] Connection state: ${state}`);

        if (state === "connected") {
          console.log(`[${this.clientId}] Connection fully established`);
          this.connected = true;
          return true;
        }

        if (state.startsWith("failed:") || state === "disconnected") {
          console.error(
            `[${this.clientId}] Connection failed with state: ${state}`,
          );
          return false;
        }

        await new Promise((resolve) => setTimeout(resolve, pollIntervalMs));
        waited += pollIntervalMs;
      }

      // If we didn't reach "connected", check final state
      const finalState = await this.tonk.getConnectionState();
      console.log(
        `[${this.clientId}] Final connection state after timeout: ${finalState}`,
      );

      // Accept "open" state as connected for now (sync may still work)
      if (finalState === "open" || finalState === "connected") {
        this.connected = true;
        return true;
      }

      return false;
    } catch (error) {
      console.error(
        `[${this.clientId}] Failed to connect to WebSocket:`,
        error,
      );
      return false;
    }
  }

  async createDocument(filePath, content) {
    if (!this.tonk) {
      throw new Error(`[${this.clientId}] No TonkCore instance available`);
    }

    try {
      console.log(`[${this.clientId}] Creating document at path: ${filePath}`);
      console.log(`[${this.clientId}] Content: ${content}`);

      // VFS stores JSON objects, so we wrap the content
      await this.tonk.createFile(filePath, { content });

      console.log(
        `[${this.clientId}] Document created successfully at: ${filePath}`,
      );

      return { filePath, content };
    } catch (error) {
      console.error(`[${this.clientId}] Failed to create document:`, error);
      throw error;
    }
  }

  async readDocument(filePath) {
    if (!this.tonk) {
      throw new Error(`[${this.clientId}] No TonkCore instance available`);
    }

    try {
      console.log(`[${this.clientId}] Reading document at path: ${filePath}`);

      // Check if file exists first
      const exists = await this.tonk.exists(filePath);
      if (!exists) {
        console.log(
          `[${this.clientId}] Document not found at path: ${filePath}`,
        );
        return null;
      }

      const doc = await this.tonk.readFile(filePath);

      if (doc && doc.content) {
        console.log(`[${this.clientId}] Found document at: ${filePath}`);
        const content = doc.content.content; // unwrap our content wrapper
        console.log(`[${this.clientId}] Content: ${content}`);
        return content;
      } else {
        console.log(
          `[${this.clientId}] Document empty or malformed at: ${filePath}`,
        );
        return null;
      }
    } catch (error) {
      console.error(`[${this.clientId}] Error reading document:`, error);
      // Return null instead of throwing to match expected behavior
      return null;
    }
  }
}

// Test TonkCore VFS API directly (without WebSocket)
async function testTonkVfsAPI() {
  console.log("=== Testing TonkCore VFS API Directly ===");

  const client1 = new TonkTestClient("TONK-1");

  try {
    // Initialize client (no shared bundle needed for local test)
    console.log("Initializing client...");
    const init1 = await client1.init();

    if (!init1) {
      throw new Error("Failed to initialize client");
    }

    console.log("Testing direct VFS document creation and retrieval...");

    // Client 1 creates a document via VFS directly
    const testContent = `Hello from Client 1 via Node.js! Time: ${Date.now()}`;
    const testPath = "/test/local-doc.json";
    const { filePath, content: originalContent } = await client1.createDocument(
      testPath,
      testContent,
    );

    console.log(`Document created at path: ${filePath}`);

    // Same client tries to read back the document
    console.log("Same client attempting to read back the document...");
    const retrievedContent = await client1.readDocument(filePath);

    if (retrievedContent !== null) {
      console.log(
        "*** TonkCore VFS API working! Document was created and retrieved",
      );
      console.log("Original content:", originalContent);
      console.log("Retrieved content:", retrievedContent);

      if (retrievedContent.includes("Hello from Client 1")) {
        console.log("*** Content matches - VFS API working correctly!");
      } else {
        console.log("*** Content mismatch - VFS API issue detected");
      }
    } else {
      console.log(
        "*** TonkCore VFS API failed - could not retrieve the document",
      );
    }

    console.log("=== TonkCore VFS API Test Complete ===\n");
  } catch (error) {
    console.error("*** TonkCore VFS API test failed:", error);
    throw error;
  }
}

// Test TonkCore document sync directly
async function testTonkDirectSync() {
  console.log("=== Testing Direct TonkCore Document Sync ===");

  // Load the shared bundle - both clients need to share the same root document
  console.log(`Loading shared bundle from: ${BUNDLE_PATH}`);
  let bundleBytes;
  try {
    bundleBytes = fs.readFileSync(BUNDLE_PATH);
    console.log(`Loaded bundle: ${bundleBytes.length} bytes`);
  } catch (error) {
    console.error(`Failed to load bundle from ${BUNDLE_PATH}:`, error.message);
    throw error;
  }

  const client1 = new TonkTestClient("TONK-1");
  const client2 = new TonkTestClient("TONK-2");

  try {
    // Initialize both clients from the same bundle (shared root document)
    console.log("Initializing clients from shared bundle...");
    const init1 = await client1.init(bundleBytes);
    const init2 = await client2.init(bundleBytes);

    if (!init1 || !init2) {
      throw new Error("Failed to initialize clients");
    }

    // Verify both clients have the same root (path index) document
    const bytes1 = await client1.tonk.toBytes();
    const bytes2 = await client2.tonk.toBytes();
    const bundle1 = wasm.create_bundle_from_bytes(bytes1);
    const bundle2 = wasm.create_bundle_from_bytes(bytes2);
    const rootId1 = await bundle1.getRootId();
    const rootId2 = await bundle2.getRootId();
    console.log(`[TONK-1] Root ID: ${rootId1}`);
    console.log(`[TONK-2] Root ID: ${rootId2}`);
    if (rootId1 !== rootId2) {
      throw new Error(
        "Root document IDs differ - both clients must share the same root for sync to work",
      );
    }
    console.log("Both clients share the same root document ID");

    // Connect both clients
    console.log("Connecting clients to WebSocket...");
    const conn1 = await client1.connect();
    const conn2 = await client2.connect();

    if (!conn1 || !conn2) {
      throw new Error("Failed to connect clients");
    }

    console.log(
      "Both clients connected, testing direct TonkCore document creation...",
    );

    // Client 1 creates a document via TonkCore directly
    const testContent = `Hello from Client 1 via Node.js! Time: ${Date.now()}`;
    const testPath = "/sync-test/shared-doc.json";
    const { filePath, content: originalContent } = await client1.createDocument(
      testPath,
      testContent,
    );

    console.log(`Document created at path: ${filePath}`);

    // Wait for sync propagation (poll until document appears or timeout)
    console.log("Waiting for sync propagation...");
    const maxWaitMs = 5000;
    const pollIntervalMs = 250;
    let syncedInTime = false;

    for (let waited = 0; waited < maxWaitMs; waited += pollIntervalMs) {
      await new Promise((resolve) => setTimeout(resolve, pollIntervalMs));
      const exists = await client2.tonk.exists(testPath);
      if (exists) {
        console.log(
          `Document synced to Client 2 in ${waited + pollIntervalMs}ms`,
        );
        syncedInTime = true;
        break;
      }
    }

    if (!syncedInTime) {
      console.log(`Document did not sync within ${maxWaitMs}ms`);
    }

    // Client 2 tries to read the document
    console.log("Client 2 attempting to read the document...");
    const syncedContent = await client2.readDocument(filePath);

    if (syncedContent !== null) {
      console.log(
        "*** TonkCore Document sync successful! Client 2 found the document",
      );
      console.log("Original content (Client 1):", originalContent);
      console.log("Synced content (Client 2):", syncedContent);

      if (syncedContent.includes("Hello from Client 1")) {
        console.log("*** Content matches - sync working correctly!");
      } else {
        console.log("*** Content mismatch - sync issue detected");
      }
    } else {
      console.log(
        "*** TonkCore Document sync failed - Client 2 could not find the document",
      );
      console.log(
        "This indicates that TonkCore may not be syncing documents properly",
      );
    }

    console.log("=== TonkCore Direct Sync Test Complete ===\n");
  } catch (error) {
    console.error("*** TonkCore sync test failed:", error);
    throw error;
  }
}

module.exports = { TonkTestClient, testTonkDirectSync, testTonkVfsAPI };

// Run test if this file is executed directly
if (require.main === module) {
  // First test the VFS API directly, then test sync
  testTonkVfsAPI()
    .then(() => testTonkDirectSync())
    .then(() => {
      console.log("TonkCore tests completed");
      process.exit(0);
    })
    .catch((error) => {
      console.error("Test failed:", error);
      process.exit(1);
    });
}

#!/usr/bin/env node

// Import the WASM module using CommonJS pattern (required for wasm-bindgen output)
import pkg from "../../pkg-node/tonk_core.js";
const { create_tonk, create_tonk_with_peer_id } = pkg;

class TonkTestClient {
  constructor(clientId, serverUrl = "ws://127.0.0.1:8082") {
    this.clientId = clientId;
    this.serverUrl = serverUrl;
    this.tonk = null;
    this.connected = false;
  }

  async init() {
    try {
      console.log(`[${this.clientId}] Creating TonkCore...`);
      this.tonk = await create_tonk_with_peer_id(this.clientId);

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
      console.log(`[${this.clientId}] Connected to WebSocket successfully`);
      this.connected = true;

      // Wait a moment for connection to stabilize
      await new Promise((resolve) => setTimeout(resolve, 500));
      return true;
    } catch (error) {
      console.error(
        `[${this.clientId}] Failed to connect to WebSocket:`,
        error,
      );
      return false;
    }
  }

  async createDocument(
    path,
    content = `Hello from ${this.clientId} at ${Date.now()}!`,
  ) {
    if (!this.tonk) {
      throw new Error(`[${this.clientId}] No TonkCore available`);
    }

    try {
      console.log(`[${this.clientId}] Creating document at path: ${path}`);
      console.log(`[${this.clientId}] Content: ${content}`);

      // Create a document using TonkCore VFS
      await this.tonk.createFile(path, content);

      console.log(`[${this.clientId}] Document created successfully at: ${path}`);

      return { path, content };
    } catch (error) {
      console.error(`[${this.clientId}] Failed to create document:`, error);
      throw error;
    }
  }

  async readDocument(path) {
    if (!this.tonk) {
      throw new Error(`[${this.clientId}] No TonkCore available`);
    }

    try {
      console.log(`[${this.clientId}] Reading document at path: ${path}`);
      
      const exists = await this.tonk.exists(path);
      if (!exists) {
        console.log(`[${this.clientId}] Document not found at path: ${path}`);
        return null;
      }

      const result = await this.tonk.readFile(path);

      if (result !== null) {
        console.log(`[${this.clientId}] Found document at path: ${path}`);
        // Handle wrapped primitives
        const content = result.content?.value !== undefined 
          ? result.content.value 
          : result.content;
        console.log(`[${this.clientId}] Content:`, content);
        return content;
      } else {
        console.log(`[${this.clientId}] Document not found at path: ${path}`);
        return null;
      }
    } catch (error) {
      console.error(`[${this.clientId}] Error reading document:`, error);
      return null;
    }
  }
}

// Test TonkCore VFS API directly (without WebSocket)
async function testTonkVfsAPI() {
  console.log("=== Testing TonkCore VFS API Directly ===");

  const client1 = new TonkTestClient("TONK-1");

  try {
    // Initialize client
    console.log("Initializing client...");
    const init1 = await client1.init();

    if (!init1) {
      throw new Error("Failed to initialize client");
    }

    console.log("Testing direct TonkCore VFS document creation and retrieval...");

    // Client 1 creates a document via TonkCore VFS
    const testPath = "/test-doc.txt";
    const testContent = `Hello from Client 1 via Node.js! Time: ${Date.now()}`;
    const { path, content: originalContent } = await client1.createDocument(
      testPath,
      testContent,
    );

    console.log(`Document created at path: ${path}`);

    // Same client tries to read back the document
    console.log("Same client attempting to read back the document...");
    const retrievedContent = await client1.readDocument(path);

    if (retrievedContent !== null) {
      console.log(
        "TonkCore VFS API working! Document was created and retrieved",
      );
      console.log("Original content:", originalContent);
      console.log("Retrieved content:", retrievedContent);

      if (retrievedContent.includes && retrievedContent.includes("Hello from Client 1")) {
        console.log("Content matches - VFS API working correctly!");
      } else if (retrievedContent === originalContent) {
        console.log("Content matches exactly - VFS API working correctly!");
      } else {
        console.log("Content mismatch - VFS API issue detected");
      }
    } else {
      console.log("TonkCore VFS API failed - could not retrieve the document");
    }

    console.log("=== TonkCore VFS API Test Complete ===\n");
  } catch (error) {
    console.error("TonkCore VFS API test failed:", error);
    throw error;
  }
}

// Test TonkCore document sync via WebSocket
async function testSamodDirectSync() {
  console.log("=== Testing TonkCore Document Sync via WebSocket ===");

  const client1 = new TonkTestClient("TONK-SYNC-1");
  const client2 = new TonkTestClient("TONK-SYNC-2");

  try {
    // Initialize both clients
    console.log("Initializing clients...");
    const init1 = await client1.init();
    const init2 = await client2.init();

    if (!init1 || !init2) {
      throw new Error("Failed to initialize clients");
    }

    // Connect both clients
    console.log("Connecting clients to WebSocket...");
    const conn1 = await client1.connect();
    const conn2 = await client2.connect();

    if (!conn1 || !conn2) {
      throw new Error("Failed to connect clients");
    }

    console.log(
      "Both clients connected, testing TonkCore document creation...",
    );

    // Client 1 creates a document
    const testPath = "/sync-test.txt";
    const testContent = `Hello from Client 1 via Node.js! Time: ${Date.now()}`;
    const { path, content: originalContent } = await client1.createDocument(
      testPath,
      testContent,
    );

    console.log(`Document created at path: ${path}`);

    // Wait for sync propagation
    console.log("Waiting for sync propagation...");
    await new Promise((resolve) => setTimeout(resolve, 3000));

    // Client 2 tries to read the document
    console.log("Client 2 attempting to read the document...");
    const syncedContent = await client2.readDocument(path);

    if (syncedContent !== null) {
      console.log(
        "Document sync successful! Client 2 found the document",
      );
      console.log("Original content (Client 1):", originalContent);
      console.log("Synced content (Client 2):", syncedContent);

      if (syncedContent.includes && syncedContent.includes("Hello from Client 1")) {
        console.log("Content matches - sync working correctly!");
      } else if (syncedContent === originalContent) {
        console.log("Content matches exactly - sync working correctly!");
      } else {
        console.log("Content mismatch - sync issue detected");
      }
    } else {
      console.log(
        "Document sync failed - Client 2 could not find the document",
      );
      console.log(
        "This indicates that sync may not be propagating documents properly",
      );
    }

    console.log("=== TonkCore Sync Test Complete ===\n");
  } catch (error) {
    console.error("TonkCore sync test failed:", error);
    throw error;
  }
}

export { TonkTestClient, testSamodDirectSync, testTonkVfsAPI };

// Run test if this file is executed directly
if (process.argv[1] === new URL(import.meta.url).pathname) {
  // First test the VFS API directly
  testTonkVfsAPI()
    .then(() => testSamodDirectSync())
    .then(() => {
      console.log("TonkCore tests completed");
      process.exit(0);
    })
    .catch((error) => {
      console.error("Test failed:", error);
      process.exit(1);
    });
}

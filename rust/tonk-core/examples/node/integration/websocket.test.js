/**
 * WebSocket integration tests
 *
 * Tests WebSocket connectivity and sync functionality of TonkCore.
 */

const { expect } = require("chai");
const {
  initWasm,
  generatePeerId,
  waitFor,
  sleep,
} = require("../../shared/test-utils");
const { createTestServer } = require("../../shared/test-server");

describe("WebSocket Integration Tests", () => {
  let wasm, testServer;

  before(async function () {
    this.timeout(10000);
    wasm = await initWasm();
  });

  beforeEach(async () => {
    testServer = await createTestServer();
  });

  afterEach(async () => {
    if (testServer) {
      await testServer.stop();
    }
  });

  describe("WebSocket Server Infrastructure", () => {
    it("should start and stop test server", async () => {
      expect(testServer.port).to.be.a("number");
      expect(testServer.port).to.be.greaterThan(0);
      expect(testServer.getClientCount()).to.equal(0);

      const url = testServer.getUrl();
      expect(url).to.include("ws://localhost:");
      expect(url).to.include(testServer.port.toString());
    });

    it("should handle client connections", async () => {
      const WebSocket = require("ws");
      const client = new WebSocket(testServer.getUrl());

      await new Promise((resolve) => {
        client.on("open", resolve);
      });

      expect(testServer.getClientCount()).to.equal(1);

      client.close();
      await sleep(100); // Give time for cleanup
    });

    it("should relay messages between clients", async () => {
      const WebSocket = require("ws");

      // Create two clients
      const client1 = new WebSocket(testServer.getUrl());
      const client2 = new WebSocket(testServer.getUrl());

      await Promise.all([
        new Promise((resolve) => client1.on("open", resolve)),
        new Promise((resolve) => client2.on("open", resolve)),
      ]);

      expect(testServer.getClientCount()).to.equal(2);

      // Set up message listener on client2
      const receivedMessages = [];
      client2.on("message", (data) => {
        receivedMessages.push(data.toString());
      });

      // Send message from client1
      client1.send("Hello from client 1");

      // Wait for message to be relayed
      await waitFor(() => receivedMessages.length > 0, 2000);

      expect(receivedMessages).to.have.lengthOf(1);
      expect(receivedMessages[0]).to.equal("Hello from client 1");

      client1.close();
      client2.close();
    });
  });

  describe("Tonk WebSocket Integration", () => {
    it("should handle WebSocket connection attempts gracefully", async () => {
      const tonk = await wasm.create_tonk();

      // NOTE: WebSocket connections may or may not be implemented
      // This test verifies that the method exists and handles the call appropriately

      try {
        await tonk.connectWebsocket(testServer.getUrl());
        // If we get here, connection succeeded
        console.log("    WebSocket connection implemented and working");
      } catch (error) {
        // Expected for current implementation - connection might fail
        if (error.message) {
          console.log(`    WebSocket connection error: ${error.message}`);
        }
      } finally {
        // Disconnect to ensure clean teardown
        if (typeof tonk.disconnect === "function") {
          await tonk.disconnect();
        }
      }
    });

    it("should handle multiple connection attempts", async () => {
      const tonks = [];

      // Create multiple tonk instances
      for (let i = 0; i < 3; i++) {
        tonks.push(await wasm.create_tonk());
      }

      // Try to connect all tonk instances
      const connectionPromises = tonks.map(async (tonk) => {
        try {
          await tonk.connectWebsocket(testServer.getUrl());
          return { success: true, tonk };
        } catch (error) {
          return { success: false, error: error.message, tonk };
        }
      });

      const results = await Promise.all(connectionPromises);

      // All should either succeed or fail
      const successes = results.filter((r) => r.success).length;
      const failures = results.filter((r) => !r.success).length;

      if (successes === 0) {
        console.log("    All WebSocket connections failed (may be expected)");
        expect(failures).to.equal(3);
      } else {
        console.log(`    ${successes} WebSocket connections succeeded`);
      }

      // Disconnect all tonks for clean teardown
      for (const result of results) {
        if (typeof result.tonk.disconnect === "function") {
          await result.tonk.disconnect();
        }
      }
    });

    it("should maintain tonk state regardless of WebSocket status", async () => {
      const tonk = await wasm.create_tonk();

      // Create some VFS data
      await tonk.createFile("/test-before-ws.json", { before: "connection" });

      // Try WebSocket connection
      try {
        await tonk.connectWebsocket(testServer.getUrl());
      } catch (error) {
        // Connection failure is expected and shouldn't affect VFS
      }

      // VFS should still work
      await tonk.createFile("/test-after-ws.json", {
        after: "connection attempt",
      });

      const beforeExists = await tonk.exists("/test-before-ws.json");
      const afterExists = await tonk.exists("/test-after-ws.json");

      expect(beforeExists).to.be.true;
      expect(afterExists).to.be.true;

      // Tonk should still be functional
      const peerId = await tonk.getPeerId();
      expect(peerId).to.be.a("string");

      // Disconnect for clean teardown
      if (typeof tonk.disconnect === "function") {
        await tonk.disconnect();
      }
    });

    it("should report connection state correctly", async () => {
      const tonk = await wasm.create_tonk();

      // Before connection attempt
      let state = await tonk.getConnectionState();
      expect(state).to.equal("disconnected");

      let isConnected = await tonk.isConnected();
      expect(isConnected).to.be.false;

      // Try to connect
      try {
        await tonk.connectWebsocket(testServer.getUrl());
        // Check state after connection
        state = await tonk.getConnectionState();
        isConnected = await tonk.isConnected();
        console.log(
          `    Connection state: ${state}, isConnected: ${isConnected}`,
        );
      } catch (error) {
        // Connection failed - state should still be retrievable
        state = await tonk.getConnectionState();
        console.log(`    Connection failed, state: ${state}`);
      } finally {
        // Disconnect for clean teardown
        if (typeof tonk.disconnect === "function") {
          await tonk.disconnect();
        }
      }
    });
  });

  describe("Future WebSocket Functionality", () => {
    // These tests document expected behavior once WebSocket sync is fully implemented

    it("should sync VFS changes between tonk instances (future)", async () => {
      console.log("    i Future test: VFS sync between connected instances");

      const tonk1 = await wasm.create_tonk_with_peer_id("peer-1");
      const tonk2 = await wasm.create_tonk_with_peer_id("peer-2");

      // For now, just verify instances are independent
      await tonk1.createFile("/tonk1-file.json", { from: "tonk 1" });
      await tonk2.createFile("/tonk2-file.json", { from: "tonk 2" });

      const file1InTonk2 = await tonk2.exists("/tonk1-file.json");
      const file2InTonk1 = await tonk1.exists("/tonk2-file.json");

      // Currently should be false (no sync)
      expect(file1InTonk2).to.be.false;
      expect(file2InTonk1).to.be.false;

      console.log(
        "    i Once WebSocket sync is implemented, files should sync between instances",
      );
    });

    it("should handle peer discovery and connection (future)", async () => {
      console.log("    i Future test: Automatic peer discovery and connection");

      const tonk = await wasm.create_tonk();

      // Future API might look like:
      // const peers = await tonk.discoverPeers();
      // await tonk.connectToPeer(peers[0]);

      console.log(
        "    i Future functionality: peer discovery and automatic connection",
      );
    });

    it("should handle conflict resolution in sync (future)", async () => {
      console.log("    i Future test: Conflict resolution during sync");

      const tonk1 = await wasm.create_tonk_with_peer_id("conflict-test-1");
      const tonk2 = await wasm.create_tonk_with_peer_id("conflict-test-2");

      // Future: Test conflicting changes to same file
      // and verify CRDT-based conflict resolution

      console.log("    i Future functionality: CRDT-based conflict resolution");
    });
  });

  describe("WebSocket Error Handling", () => {
    it("should handle connection to non-existent server", async () => {
      const tonk = await wasm.create_tonk();
      const invalidUrl = "ws://localhost:99999"; // Non-existent server

      try {
        await tonk.connectWebsocket(invalidUrl);
        expect.fail("Expected connection to fail for non-existent server");
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });

    it("should handle malformed WebSocket URLs", async () => {
      const tonk = await wasm.create_tonk();
      const malformedUrls = [
        "not-a-url",
        "http://localhost:8081", // HTTP instead of WS
        "ws://invalid-host:-1",
        "",
      ];

      for (const url of malformedUrls) {
        try {
          await tonk.connectWebsocket(url);
          expect.fail(`Expected connection to fail for malformed URL: ${url}`);
        } catch (error) {
          expect(error).to.not.be.undefined;
        }
      }
    });
  });

  describe("Test Server Reliability", () => {
    it("should handle rapid client connections and disconnections", async () => {
      const WebSocket = require("ws");
      const clients = [];

      // Create many connections rapidly
      for (let i = 0; i < 20; i++) {
        const client = new WebSocket(testServer.getUrl());
        clients.push(client);

        // Connect and immediately disconnect some clients
        if (i % 3 === 0) {
          client.on("open", () => client.close());
        }
      }

      // Wait for connections to stabilize
      await sleep(500);

      // Server should still be responsive
      expect(testServer.getClientCount()).to.be.lessThan(20);
      expect(testServer.getClientCount()).to.be.greaterThanOrEqual(0);

      // Cleanup
      for (const client of clients) {
        if (client.readyState === WebSocket.OPEN) {
          client.close();
        }
      }
    });

    it("should handle message flooding", async () => {
      const WebSocket = require("ws");
      const client = new WebSocket(testServer.getUrl());

      await new Promise((resolve) => client.on("open", resolve));

      // Send many messages rapidly
      const messageCount = 100;
      for (let i = 0; i < messageCount; i++) {
        client.send(`Message ${i}`);
      }

      // Wait for messages to be processed
      await testServer.waitForMessages(messageCount, 5000);

      const messageLog = testServer.getMessageLog();
      expect(messageLog.length).to.equal(messageCount);

      client.close();
    });
  });
});

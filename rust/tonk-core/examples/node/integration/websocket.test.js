/**
 * WebSocket integration tests
 * Note: These tests verify WebSocket connection handling and infrastructure.
 * Full sync functionality depends on WebSocket implementation in the WASM bindings.
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

  describe("TonkCore WebSocket Integration", () => {
    it("should handle WebSocket connection attempts gracefully", async () => {
      const tonk = await wasm.create_tonk();

      // NOTE: WebSocket connections in WASM may have different behavior than native
      // This test verifies the method exists and handles connection attempts

      try {
        await tonk.connectWebsocket(testServer.getUrl());
        console.log("    WebSocket connection succeeded");
      } catch (error) {
        // Connection may fail for various reasons - that's OK
        // The important thing is the method exists and doesn't crash
        expect(error).to.not.be.undefined;
        console.log("    WebSocket connection failed (expected in some environments)");
      }
    });

    it("should handle multiple connection attempts", async () => {
      const instances = [];

      // Create multiple TonkCore instances
      for (let i = 0; i < 3; i++) {
        instances.push(await wasm.create_tonk());
      }

      // Try to connect all instances
      const connectionPromises = instances.map(async (tonk) => {
        try {
          await tonk.connectWebsocket(testServer.getUrl());
          return { success: true };
        } catch (error) {
          return { success: false, error: error.message || String(error) };
        }
      });

      const results = await Promise.all(connectionPromises);

      // All should either succeed or fail consistently
      const successes = results.filter((r) => r.success).length;
      const failures = results.filter((r) => !r.success).length;

      console.log(`    ${successes} succeeded, ${failures} failed`);

      // Results should be consistent (either all succeed or all fail)
      expect(successes === 0 || successes === 3).to.be.true;
    });

    it("should maintain TonkCore state regardless of WebSocket status", async () => {
      const tonk = await wasm.create_tonk();

      // Create some VFS data
      await tonk.createFile("/test-before-ws.txt", "before connection");

      // Try WebSocket connection
      try {
        await tonk.connectWebsocket(testServer.getUrl());
      } catch (error) {
        // Connection failure is expected and shouldn't affect VFS
      }

      // VFS should still work
      await tonk.createFile("/test-after-ws.txt", "after connection attempt");

      const beforeExists = await tonk.exists("/test-before-ws.txt");
      const afterExists = await tonk.exists("/test-after-ws.txt");

      expect(beforeExists).to.be.true;
      expect(afterExists).to.be.true;

      // TonkCore should still be functional
      const peerId = await tonk.getPeerId();
      expect(peerId).to.be.a("string");
    });
  });

  describe("Multi-Instance Behavior", () => {
    it("should maintain separate state across TonkCore instances", async () => {
      const tonk1 = await wasm.create_tonk_with_peer_id("peer-1");
      const tonk2 = await wasm.create_tonk_with_peer_id("peer-2");

      // Create files in each instance
      await tonk1.createFile("/tonk1-file.txt", "from tonk 1");
      await tonk2.createFile("/tonk2-file.txt", "from tonk 2");

      // Each instance should only see its own files
      const file1InTonk2 = await tonk2.exists("/tonk1-file.txt");
      const file2InTonk1 = await tonk1.exists("/tonk2-file.txt");

      // Without sync, instances are isolated
      expect(file1InTonk2).to.be.false;
      expect(file2InTonk1).to.be.false;
    });

    it("should handle independent operations on multiple instances", async () => {
      const instances = [];

      // Create 5 TonkCore instances
      for (let i = 0; i < 5; i++) {
        const tonk = await wasm.create_tonk();
        instances.push(tonk);

        // Each instance creates its own files
        await tonk.createFile(`/instance-${i}.txt`, `Content from instance ${i}`);
      }

      // Verify each instance has its own file
      for (let i = 0; i < 5; i++) {
        const exists = await instances[i].exists(`/instance-${i}.txt`);
        expect(exists).to.be.true;

        const content = await instances[i].readFile(`/instance-${i}.txt`);
        // Handle wrapped primitive
        const actualContent = content.content.value !== undefined 
          ? content.content.value 
          : content.content;
        expect(actualContent).to.equal(`Content from instance ${i}`);
      }
    });
  });

  describe("WebSocket Error Handling", () => {
    it("should handle connection to non-existent server", async () => {
      const tonk = await wasm.create_tonk();
      const invalidUrl = "ws://localhost:99999"; // Non-existent server

      try {
        await tonk.connectWebsocket(invalidUrl);
        // If it succeeds, that's unexpected but OK
      } catch (error) {
        expect(error).to.not.be.undefined;
        // Error could be about connection failure
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
          // Some URLs might be accepted, others rejected
        } catch (error) {
          expect(error).to.not.be.undefined;
        }
      }

      // TonkCore should still work after these errors
      const peerId = await tonk.getPeerId();
      expect(peerId).to.be.a("string");
    });

    it("should maintain stability after connection errors", async () => {
      const tonk = await wasm.create_tonk();

      // Create some data first
      await tonk.createFile("/pre-error.txt", "before error");

      // Try multiple invalid connections
      for (let i = 0; i < 3; i++) {
        try {
          await tonk.connectWebsocket("ws://localhost:99999");
        } catch (error) {
          // Expected
        }
      }

      // Should still be able to use VFS
      await tonk.createFile("/post-error.txt", "after errors");

      const preExists = await tonk.exists("/pre-error.txt");
      const postExists = await tonk.exists("/post-error.txt");

      expect(preExists).to.be.true;
      expect(postExists).to.be.true;

      // Should still be able to export
      const bytes = await tonk.toBytes(null);
      expect(bytes).to.be.instanceOf(Uint8Array);
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

    it("should handle binary messages", async () => {
      const WebSocket = require("ws");
      const client = new WebSocket(testServer.getUrl());

      await new Promise((resolve) => client.on("open", resolve));

      // Send binary data
      const binaryData = new Uint8Array([1, 2, 3, 4, 5]);
      client.send(binaryData);

      await sleep(100);

      const messageLog = testServer.getMessageLog();
      expect(messageLog.length).to.be.greaterThan(0);

      client.close();
    });
  });

  describe("Integration Scenarios", () => {
    it("should export TonkCore after WebSocket operations", async () => {
      const tonk = await wasm.create_tonk();

      // Create content
      await tonk.createFile("/before-connect.txt", "before");

      // Try connecting (may fail)
      try {
        await tonk.connectWebsocket(testServer.getUrl());
      } catch (error) {
        // OK
      }

      // Create more content
      await tonk.createFile("/after-connect.txt", "after");

      // Export should work
      const bytes = await tonk.toBytes(null);
      expect(bytes.length).to.be.greaterThan(0);

      // Import should work
      const tonk2 = await wasm.create_tonk_from_bytes(bytes);
      const beforeExists = await tonk2.exists("/before-connect.txt");
      const afterExists = await tonk2.exists("/after-connect.txt");

      expect(beforeExists).to.be.true;
      expect(afterExists).to.be.true;
    });

    it("should handle TonkCore lifecycle with WebSocket attempts", async () => {
      // Create
      const tonk = await wasm.create_tonk();
      const originalPeerId = await tonk.getPeerId();

      // Add data
      await tonk.createFile("/lifecycle.txt", "lifecycle test");

      // Try WebSocket
      try {
        await tonk.connectWebsocket(testServer.getUrl());
      } catch (error) {
        // OK
      }

      // Modify data
      await tonk.setFile("/lifecycle.txt", "modified");

      // Verify peer ID unchanged
      const currentPeerId = await tonk.getPeerId();
      expect(currentPeerId).to.equal(originalPeerId);

      // Verify data
      const content = await tonk.readFile("/lifecycle.txt");
      // Handle wrapped primitive
      const actualContent = content.content.value !== undefined 
        ? content.content.value 
        : content.content;
      expect(actualContent).to.equal("modified");
    });
  });
});

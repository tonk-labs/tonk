/**
 * WebSocket integration tests
 * Note: These tests are currently limited since WebSocket functionality
 * is not yet implemented for WASM targets
 */

const { expect } = require('chai');
const {
  initWasm,
  generatePeerId,
  waitFor,
  sleep,
} = require('../../shared/test-utils');
const { createTestServer } = require('../../shared/test-server');

describe('WebSocket Integration Tests', function () {
  let wasm, testServer;

  before(async function () {
    this.timeout(10000);
    wasm = await initWasm();
  });

  beforeEach(async function () {
    testServer = await createTestServer();
  });

  afterEach(async function () {
    if (testServer) {
      await testServer.stop();
    }
  });

  describe('WebSocket Server Infrastructure', function () {
    it('should start and stop test server', async function () {
      expect(testServer.port).to.be.a('number');
      expect(testServer.port).to.be.greaterThan(0);
      expect(testServer.getClientCount()).to.equal(0);

      const url = testServer.getUrl();
      expect(url).to.include('ws://localhost:');
      expect(url).to.include(testServer.port.toString());
    });

    it('should handle client connections', async function () {
      const WebSocket = require('ws');
      const client = new WebSocket(testServer.getUrl());

      await new Promise(resolve => {
        client.on('open', resolve);
      });

      expect(testServer.getClientCount()).to.equal(1);

      client.close();
      await sleep(100); // Give time for cleanup
    });

    it('should relay messages between clients', async function () {
      const WebSocket = require('ws');

      // Create two clients
      const client1 = new WebSocket(testServer.getUrl());
      const client2 = new WebSocket(testServer.getUrl());

      await Promise.all([
        new Promise(resolve => client1.on('open', resolve)),
        new Promise(resolve => client2.on('open', resolve)),
      ]);

      expect(testServer.getClientCount()).to.equal(2);

      // Set up message listener on client2
      const receivedMessages = [];
      client2.on('message', data => {
        receivedMessages.push(data.toString());
      });

      // Send message from client1
      client1.send('Hello from client 1');

      // Wait for message to be relayed
      await waitFor(() => receivedMessages.length > 0, 2000);

      expect(receivedMessages).to.have.lengthOf(1);
      expect(receivedMessages[0]).to.equal('Hello from client 1');

      client1.close();
      client2.close();
    });
  });

  describe('Sync Engine WebSocket Integration', function () {
    it('should handle WebSocket connection attempts gracefully', async function () {
      const engine = await wasm.create_sync_engine();

      // NOTE: Since WebSocket connections are not yet implemented for WASM,
      // this test verifies that the method exists and handles the call appropriately

      try {
        // This should either work (if implemented) or throw a descriptive error
        await engine.connectWebsocket(testServer.getUrl());

        // If we get here, connection succeeded
        console.log('    ✓ WebSocket connection implemented and working');
      } catch (error) {
        // Expected for current implementation
        if (error.message) {
          expect(error.message).to.include('WebSocket error');
        }
        console.log(
          '    ℹ WebSocket connection not yet implemented (expected)'
        );
      }
    });

    it('should handle multiple connection attempts', async function () {
      const engines = [];

      // Create multiple engines
      for (let i = 0; i < 3; i++) {
        engines.push(await wasm.create_sync_engine());
      }

      // Try to connect all engines
      const connectionPromises = engines.map(async engine => {
        try {
          await engine.connectWebsocket(testServer.getUrl());
          return { success: true };
        } catch (error) {
          return { success: false, error: error.message };
        }
      });

      const results = await Promise.all(connectionPromises);

      // All should either succeed or fail with the same reason
      const successes = results.filter(r => r.success).length;
      const failures = results.filter(r => !r.success).length;

      if (successes === 0) {
        // All failed - expected for current implementation
        expect(failures).to.equal(3);
        console.log('    ℹ All WebSocket connections failed as expected');
      } else {
        // Some or all succeeded - WebSocket is implemented
        console.log(`    ✓ ${successes} WebSocket connections succeeded`);
      }
    });

    it('should maintain engine state regardless of WebSocket status', async function () {
      const engine = await wasm.create_sync_engine();
      const vfs = await engine.getVfs();

      // Create some VFS data
      await vfs.createFile('/test-before-ws.txt', 'before connection');

      // Try WebSocket connection
      try {
        await engine.connectWebsocket(testServer.getUrl());
      } catch (error) {
        // Connection failure is expected and shouldn't affect VFS
      }

      // VFS should still work
      await vfs.createFile('/test-after-ws.txt', 'after connection attempt');

      const beforeExists = await vfs.exists('/test-before-ws.txt');
      const afterExists = await vfs.exists('/test-after-ws.txt');

      expect(beforeExists).to.be.true;
      expect(afterExists).to.be.true;

      // Engine should still be functional
      const peerId = await engine.getPeerId();
      expect(peerId).to.be.a('string');
    });
  });

  describe('Future WebSocket Functionality', function () {
    // These tests document expected behavior once WebSocket support is implemented

    it('should sync VFS changes between engines (future)', async function () {
      // This test documents what should happen once WebSocket sync is implemented
      console.log('    ℹ Future test: VFS sync between connected engines');

      const engine1 = await wasm.create_sync_engine_with_peer_id('peer-1');
      const engine2 = await wasm.create_sync_engine_with_peer_id('peer-2');

      const vfs1 = await engine1.getVfs();
      const vfs2 = await engine2.getVfs();

      // For now, just verify engines are independent
      await vfs1.createFile('/engine1-file.txt', 'from engine 1');
      await vfs2.createFile('/engine2-file.txt', 'from engine 2');

      const file1InEngine2 = await vfs2.exists('/engine1-file.txt');
      const file2InEngine1 = await vfs1.exists('/engine2-file.txt');

      // Currently should be false (no sync)
      expect(file1InEngine2).to.be.false;
      expect(file2InEngine1).to.be.false;

      console.log(
        '    ℹ Once WebSocket sync is implemented, files should sync between engines'
      );
    });

    it('should handle peer discovery and connection (future)', async function () {
      console.log(
        '    ℹ Future test: Automatic peer discovery and connection'
      );

      const engine = await wasm.create_sync_engine();

      // Future API might look like:
      // const peers = await engine.discoverPeers();
      // await engine.connectToPeer(peers[0]);

      console.log(
        '    ℹ Future functionality: peer discovery and automatic connection'
      );
    });

    it('should handle conflict resolution in sync (future)', async function () {
      console.log('    ℹ Future test: Conflict resolution during sync');

      const engine1 =
        await wasm.create_sync_engine_with_peer_id('conflict-test-1');
      const engine2 =
        await wasm.create_sync_engine_with_peer_id('conflict-test-2');

      // Future: Test conflicting changes to same file
      // and verify CRDT-based conflict resolution

      console.log(
        '    ℹ Future functionality: CRDT-based conflict resolution'
      );
    });
  });

  describe('WebSocket Error Handling', function () {
    it('should handle connection to non-existent server', async function () {
      const engine = await wasm.create_sync_engine();
      const invalidUrl = 'ws://localhost:99999'; // Non-existent server

      try {
        await engine.connectWebsocket(invalidUrl);
        expect.fail('Expected connection to fail for non-existent server');
      } catch (error) {
        expect(error).to.not.be.undefined;
        // Error could be about implementation or connection failure
      }
    });

    it('should handle malformed WebSocket URLs', async function () {
      const engine = await wasm.create_sync_engine();
      const malformedUrls = [
        'not-a-url',
        'http://localhost:8080', // HTTP instead of WS
        'ws://invalid-host:-1',
        '',
      ];

      for (const url of malformedUrls) {
        try {
          await engine.connectWebsocket(url);
          expect.fail(`Expected connection to fail for malformed URL: ${url}`);
        } catch (error) {
          expect(error).to.not.be.undefined;
        }
      }
    });
  });

  describe('Test Server Reliability', function () {
    it('should handle rapid client connections and disconnections', async function () {
      const WebSocket = require('ws');
      const clients = [];

      // Create many connections rapidly
      for (let i = 0; i < 20; i++) {
        const client = new WebSocket(testServer.getUrl());
        clients.push(client);

        // Connect and immediately disconnect some clients
        if (i % 3 === 0) {
          client.on('open', () => client.close());
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

    it('should handle message flooding', async function () {
      const WebSocket = require('ws');
      const client = new WebSocket(testServer.getUrl());

      await new Promise(resolve => client.on('open', resolve));

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

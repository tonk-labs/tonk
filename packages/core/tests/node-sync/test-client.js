#!/usr/bin/env node

import WebSocket from 'ws';

class SimpleTestClient {
  constructor(clientId, serverUrl = 'ws://127.0.0.1:8082') {
    this.clientId = clientId;
    this.serverUrl = serverUrl;
    this.ws = null;
    this.connected = false;
    this.messageHandlers = [];
  }

  async connect() {
    return new Promise((resolve, reject) => {
      console.log(`[${this.clientId}] Connecting to ${this.serverUrl}`);

      this.ws = new WebSocket(this.serverUrl);

      this.ws.on('open', () => {
        console.log(`[${this.clientId}] Connected to server`);
        this.connected = true;
        resolve();
      });

      this.ws.on('message', (data, isBinary) => {
        const message = isBinary ? data : data.toString();
        console.log(
          `[${this.clientId}] Received: ${isBinary ? `${data.length} bytes (binary)` : message}`
        );

        // Call message handlers
        this.messageHandlers.forEach(handler => {
          try {
            handler(data, isBinary);
          } catch (error) {
            console.error(
              `[${this.clientId}] Error in message handler:`,
              error
            );
          }
        });
      });

      this.ws.on('close', (code, reason) => {
        console.log(`[${this.clientId}] Connection closed: ${code} ${reason}`);
        this.connected = false;
      });

      this.ws.on('error', error => {
        console.error(`[${this.clientId}] WebSocket error:`, error);
        reject(error);
      });
    });
  }

  send(data) {
    if (!this.connected) {
      throw new Error(`[${this.clientId}] Not connected to server`);
    }

    const isBinary = Buffer.isBuffer(data);
    console.log(
      `[${this.clientId}] Sending: ${isBinary ? `${data.length} bytes (binary)` : data}`
    );
    this.ws.send(data);
  }

  onMessage(handler) {
    this.messageHandlers.push(handler);
  }

  disconnect() {
    if (this.ws) {
      this.ws.close();
    }
  }
}

// Test basic message relay functionality
async function testBasicRelay() {
  console.log('=== Testing Basic WebSocket Relay ===');

  const client1 = new SimpleTestClient('CLIENT-1');
  const client2 = new SimpleTestClient('CLIENT-2');

  // Set up message handlers
  const client1Messages = [];
  const client2Messages = [];

  client1.onMessage((data, isBinary) => {
    client1Messages.push({ data: isBinary ? data : data.toString(), isBinary });
  });

  client2.onMessage((data, isBinary) => {
    client2Messages.push({ data: isBinary ? data : data.toString(), isBinary });
  });

  try {
    // Connect both clients
    await client1.connect();
    await client2.connect();

    console.log('Both clients connected, testing message relay...');

    // Wait a moment for connections to stabilize
    await new Promise(resolve => setTimeout(resolve, 100));

    // Test text message relay
    client1.send('Hello from client 1');
    await new Promise(resolve => setTimeout(resolve, 100));

    if (
      client2Messages.length === 1 &&
      client2Messages[0].data === 'Hello from client 1'
    ) {
      console.log('✅ Text message relay working');
    } else {
      console.log('❌ Text message relay failed');
      console.log('Client2 received:', client2Messages);
    }

    // Test binary message relay
    const binaryData = Buffer.from([0x01, 0x02, 0x03, 0x04]);
    client2.send(binaryData);
    await new Promise(resolve => setTimeout(resolve, 100));

    if (
      client1Messages.length === 1 &&
      Buffer.isBuffer(client1Messages[0].data) &&
      client1Messages[0].data.equals(binaryData)
    ) {
      console.log('✅ Binary message relay working');
    } else {
      console.log('❌ Binary message relay failed');
      console.log('Client1 received:', client1Messages);
    }

    console.log('=== Basic Relay Test Complete ===\n');
  } catch (error) {
    console.error('Test failed:', error);
  } finally {
    client1.disconnect();
    client2.disconnect();
  }
}

// Export for use in other tests
export { SimpleTestClient, testBasicRelay };

// Run basic test if this file is executed directly
if (process.argv[1] === new URL(import.meta.url).pathname) {
  testBasicRelay()
    .then(() => {
      console.log('Test completed');
      process.exit(0);
    })
    .catch(error => {
      console.error('Test failed:', error);
      process.exit(1);
    });
}

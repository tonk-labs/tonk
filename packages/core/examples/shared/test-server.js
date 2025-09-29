/**
 * Mock WebSocket server for testing sync functionality
 */

const WebSocket = require('ws');
const { EventEmitter } = require('events');

class TestServer extends EventEmitter {
  constructor(port = 0) {
    super();
    this.port = port;
    this.server = null;
    this.clients = new Map();
    this.messageLog = [];
  }

  /**
   * Start the WebSocket server
   */
  async start() {
    return new Promise((resolve, reject) => {
      this.server = new WebSocket.Server({ port: this.port }, error => {
        if (error) {
          reject(error);
          return;
        }

        this.port = this.server.address().port;
        console.log(`Test server started on port ${this.port}`);

        this.server.on('connection', (ws, req) => {
          const clientId = `client-${Date.now()}-${Math.random().toString(36).substr(2, 5)}`;
          console.log(
            `Client ${clientId} connected from ${req.socket.remoteAddress}`
          );

          this.clients.set(clientId, {
            ws,
            id: clientId,
            connectedAt: Date.now(),
          });

          ws.on('message', data => {
            const message = {
              clientId,
              data,
              timestamp: Date.now(),
              type: this.detectMessageType(data),
            };

            this.messageLog.push(message);
            this.emit('message', message);

            // Echo back to all other clients (basic sync simulation)
            this.broadcast(data, clientId);
          });

          ws.on('close', () => {
            console.log(`Client ${clientId} disconnected`);
            this.clients.delete(clientId);
            this.emit('clientDisconnected', clientId);
          });

          ws.on('error', error => {
            console.error(`Error from client ${clientId}:`, error);
            this.emit('clientError', { clientId, error });
          });

          this.emit('clientConnected', clientId);
        });

        resolve();
      });
    });
  }

  /**
   * Stop the server
   */
  async stop() {
    return new Promise(resolve => {
      if (!this.server) {
        resolve();
        return;
      }

      // Close all client connections
      for (const [clientId, client] of this.clients) {
        client.ws.close();
      }
      this.clients.clear();

      this.server.close(() => {
        console.log(`Test server stopped`);
        resolve();
      });
    });
  }

  /**
   * Broadcast message to all clients except sender
   */
  broadcast(data, excludeClientId = null) {
    for (const [clientId, client] of this.clients) {
      if (
        clientId !== excludeClientId &&
        client.ws.readyState === WebSocket.OPEN
      ) {
        client.ws.send(data);
      }
    }
  }

  /**
   * Send message to specific client
   */
  sendToClient(clientId, data) {
    const client = this.clients.get(clientId);
    if (client && client.ws.readyState === WebSocket.OPEN) {
      client.ws.send(data);
      return true;
    }
    return false;
  }

  /**
   * Get connected client count
   */
  getClientCount() {
    return this.clients.size;
  }

  /**
   * Get all client IDs
   */
  getClientIds() {
    return Array.from(this.clients.keys());
  }

  /**
   * Get message log
   */
  getMessageLog() {
    return [...this.messageLog];
  }

  /**
   * Clear message log
   */
  clearMessageLog() {
    this.messageLog = [];
  }

  /**
   * Wait for specific number of clients to connect
   */
  async waitForClients(count, timeoutMs = 5000) {
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(
          new Error(
            `Timeout waiting for ${count} clients. Got ${this.getClientCount()}`
          )
        );
      }, timeoutMs);

      const checkClients = () => {
        if (this.getClientCount() >= count) {
          clearTimeout(timeout);
          resolve();
        }
      };

      // Check immediately
      checkClients();

      // Listen for new connections
      this.on('clientConnected', checkClients);
    });
  }

  /**
   * Wait for specific number of messages
   */
  async waitForMessages(count, timeoutMs = 5000) {
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(
          new Error(
            `Timeout waiting for ${count} messages. Got ${this.messageLog.length}`
          )
        );
      }, timeoutMs);

      const checkMessages = () => {
        if (this.messageLog.length >= count) {
          clearTimeout(timeout);
          resolve(this.getMessageLog());
        }
      };

      // Check immediately
      checkMessages();

      // Listen for new messages
      this.on('message', checkMessages);
    });
  }

  /**
   * Detect message type (basic heuristic)
   */
  detectMessageType(data) {
    try {
      if (data instanceof Buffer) {
        // Check if it looks like JSON
        const str = data.toString('utf8');
        JSON.parse(str);
        return 'json';
      }
      return 'binary';
    } catch {
      return 'binary';
    }
  }

  /**
   * Simulate network delay
   */
  async simulateDelay(minMs = 10, maxMs = 50) {
    const delay = Math.random() * (maxMs - minMs) + minMs;
    return new Promise(resolve => setTimeout(resolve, delay));
  }

  /**
   * Get server URL
   */
  getUrl() {
    return `ws://localhost:${this.port}`;
  }
}

/**
 * Create and start a test server
 */
async function createTestServer(port = 0) {
  const server = new TestServer(port);
  await server.start();
  return server;
}

module.exports = {
  TestServer,
  createTestServer,
};

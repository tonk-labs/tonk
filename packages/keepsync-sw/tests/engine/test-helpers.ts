import {WebSocketServer, WebSocket} from 'ws';

export class TestWebSocketServer {
  private wss: WebSocketServer;
  private connections: Set<WebSocket> = new Set();

  constructor(port: number) {
    this.wss = new WebSocketServer({port});
    console.log(`WebSocket server started on port ${port}`);

    this.wss.on('connection', (ws: WebSocket) => {
      // console.log('New client connected');
      this.connections.add(ws);

      ws.on('message', data => {
        // console.log('Received message:', data.toString());
        // Broadcast message to all other clients
        this.connections.forEach(client => {
          if (client !== ws && client.readyState === WebSocket.OPEN) {
            client.send(data.toString());
            // console.log('Broadcasted message to client');
          }
        });
      });

      ws.on('error', error => {
        // console.error('WebSocket client error:', error);
      });

      ws.on('close', () => {
        console.log('Client disconnected');
        this.connections.delete(ws);
      });
    });

    this.wss.on('error', error => {
      console.error('WebSocket server error:', error);
    });
  }

  get connectionCount(): number {
    return this.connections.size;
  }

  close() {
    console.log(`Closing server with ${this.connectionCount} connections`);
    this.connections.forEach(ws => {
      try {
        ws.close();
      } catch (e) {
        console.error('Error closing connection:', e);
      }
    });
    this.connections.clear();
    return new Promise<void>(resolve => {
      this.wss.close(() => {
        console.log('Server closed');
        resolve();
      });
    });
  }
}

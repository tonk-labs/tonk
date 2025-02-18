import WebSocket, { WebSocketServer } from "ws";

export class SyncServer {
  private wss: WebSocketServer;

  constructor(port: number = 8080) {
    this.wss = new WebSocketServer({ port });

    this.wss.on('connection', (ws: WebSocket) => {
      ws.on('message', (data: string) => {
        this.wss.clients.forEach(client => {
          if (client !== ws && client.readyState === WebSocket.OPEN) {
            client.send(data);
          }
        });
      });
    });
  }

  close(): void {
    this.wss.close();
  }
}

import express from 'express';
import expressWs from 'express-ws';
import { Application } from 'express-ws';
import cors from 'cors';
import { WebSocket } from 'ws';

// Create express app with WebSocket support
const expressApp = express();
const wsInstance = expressWs(expressApp);
const app: Application = wsInstance.app;
const wss = wsInstance.getWss();
const PORT = process.env.PORT || 9000;

app.use(cors());
app.use(express.json());

// WebSocket endpoint for sync engine
app.ws('/sync', (ws: WebSocket) => {
  ws.on('message', (msg: string) => {
    // Broadcast to all clients except sender
    wss.clients.forEach((client: WebSocket) => {
      if (client !== ws && client.readyState === WebSocket.OPEN) {
        client.send(msg);
      }
    });
  });
});

app.listen(PORT, () => {
  console.log(`Server running on port ${PORT}`);
});

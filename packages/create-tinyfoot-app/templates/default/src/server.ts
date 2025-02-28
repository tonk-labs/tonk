import { WebSocketServer, WebSocket } from "ws";
import { createServer } from "http";
import chalk from "chalk";

// Create a simple HTTP server
const server = createServer();
const wss = new WebSocketServer({
  server,
  path: "/sync",
});

// Store connected clients
const connections: Set<WebSocket> = new Set();

// Handle WebSocket connections
wss.on("connection", (ws: WebSocket) => {
  console.log(chalk.green(`Client connected`));
  connections.add(ws);

  // Handle messages from clients
  ws.on("message", (data: Buffer) => {
    connections.forEach((client) => {
      if (client !== ws && client.readyState === WebSocket.OPEN) {
        client.send(data.toString());
      }
    });
  });

  // Handle client disconnection
  ws.on("close", () => {
    console.log(chalk.red(`Client disconnected`));
    connections.delete(ws);
  });

  // Handle errors
  ws.on("error", (error: Error) => {
    console.error(chalk.red(`WebSocket error:`), error);
  });
});

// Start the server
const PORT = process.env.PORT || 3030;
server.listen(PORT, () => {
  console.log(chalk.green(`Sync server running on port ${PORT}`));
});

#!/usr/bin/env node

import { WebSocketServer } from 'ws';
import { createServer } from 'http';

const PORT = 8082;

// Create HTTP server for WebSocket upgrade
const server = createServer();
const wss = new WebSocketServer({ server });

// Track connected clients
const clients = new Set();

console.log(`[SERVER] Starting samod sync test server on port ${PORT}`);

wss.on('connection', (ws, req) => {
  const clientId = `client-${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
  ws.clientId = clientId;
  
  console.log(`[SERVER] Client connected: ${clientId} (total: ${clients.size + 1})`);
  clients.add(ws);

  ws.on('message', (message, isBinary) => {
    console.log(`[SERVER] Received from ${clientId}: ${isBinary ? `${message.length} bytes (binary)` : message.toString()}`);
    
    // Relay message to all OTHER connected clients (not the sender)
    let relayCount = 0;
    clients.forEach(client => {
      if (client !== ws && client.readyState === 1) { // WebSocket.OPEN = 1
        client.send(message, { binary: isBinary });
        relayCount++;
      }
    });
    
    console.log(`[SERVER] Relayed message to ${relayCount} other clients`);
  });

  ws.on('close', () => {
    console.log(`[SERVER] Client disconnected: ${clientId} (remaining: ${clients.size - 1})`);
    clients.delete(ws);
  });

  ws.on('error', (error) => {
    console.error(`[SERVER] WebSocket error for ${clientId}:`, error);
    clients.delete(ws);
  });
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`[SERVER] Samod sync test server listening on ws://127.0.0.1:${PORT}`);
  console.log(`[SERVER] Ready to relay messages between samod clients`);
});

// Graceful shutdown
process.on('SIGINT', () => {
  console.log(`\n[SERVER] Shutting down server...`);
  wss.close(() => {
    server.close(() => {
      console.log(`[SERVER] Server stopped`);
      process.exit(0);
    });
  });
});
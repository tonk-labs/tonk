const express = require('express');
const path = require('path');
const http = require('http');
const { WebSocketServer, WebSocket } = require('ws');
const { execSync } = require('child_process');
const fs = require('fs');

// Check if the server is already running
const isServerRunning = (port) => {
  try {
    const output = execSync(`lsof -i:${port} -P -n -t`).toString().trim();
    return output !== '';
  } catch (error) {
    return false;
  }
};

// Main function
const main = () => {
  const PORT = process.env.PORT || 8080;

  // Check if server is already running
  if (isServerRunning(PORT)) {
    console.log(`\x1b[31m%s\x1b[0m`, `Error: Port ${PORT} is already in use.`);
    console.log(`\x1b[33m%s\x1b[0m`, `Try running with a different port: PORT=9090 npm run serve`);
    process.exit(1);
  }

  // Check build directory exists
  const distPath = path.join(__dirname, '..', 'dist');
  if (!fs.existsSync(distPath)) {
    console.log('\x1b[33m%s\x1b[0m', 'Build directory not found. Running build first...');
    execSync('npm run build', { stdio: 'inherit' });
  }

  console.log('\x1b[34m%s\x1b[0m', 'Starting local server...');

  // Create Express app
  const app = express();
  const server = http.createServer(app);

  // Serve static files from the dist directory
  app.use(express.static(distPath));

  // Create WebSocket server
  const wss = new WebSocketServer({
    server,
    path: "/sync",
  });

  // Store connected clients
  const connections = new Set();

  // Handle WebSocket connections
  wss.on("connection", (ws) => {
    console.log("\x1b[32m%s\x1b[0m", `Client connected`);
    connections.add(ws);

    // Handle messages from clients
    ws.on("message", (data) => {
      connections.forEach((client) => {
        if (client !== ws && client.readyState === WebSocket.OPEN) {
          client.send(data.toString());
        }
      });
    });

    // Handle client disconnection
    ws.on("close", () => {
      console.log("\x1b[31m%s\x1b[0m", `Client disconnected`);
      connections.delete(ws);
    });

    // Handle errors
    ws.on("error", (error) => {
      console.error("\x1b[31m%s\x1b[0m", `WebSocket error:`, error);
    });
  });

  // Send all requests to index.html for client-side routing
  app.get('*', (_req, res) => {
    res.sendFile(path.join(distPath, 'index.html'));
  });

  // Start the server
  server.listen(PORT, () => {
    console.log(`\x1b[32m%s\x1b[0m`, `Server running on port ${PORT}`);
    console.log(`\x1b[34m%s\x1b[0m`, `Open http://localhost:${PORT} to view your app`);
    console.log(`\x1b[34m%s\x1b[0m`, 'On your phone, connect to this server using your computer\'s local IP address');
    console.log(`\x1b[34m%s\x1b[0m`, `Use pinggy to expose this server: ssh -p 443 -R0:localhost:${PORT} a.pinggy.io`);
  });
};

// Run the main function
main();

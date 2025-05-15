import { configureSyncEngine } from "@tonk/keepsync";
import { NetworkAdapterInterface } from "@automerge/automerge-repo";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { NodeFSStorageAdapter } from "@automerge/automerge-repo-storage-nodefs";
import * as http from "http";
import dotenv from "dotenv";

// Load environment variables
dotenv.config();

// Set up global error handlers
process.on("uncaughtException", (err) => {
  console.error(`Uncaught exception: ${err.message}`);
  console.error(err.stack);
});

process.on("unhandledRejection", (reason, promise) => {
  console.error("Unhandled Rejection at:", promise);
  console.error("Reason:", reason);
});

// Log startup information
console.log(`Starting Tonk worker at ${new Date().toISOString()}`);
console.log(`Node.js version: ${process.version}`);
console.log(`Platform: ${process.platform}`);

// Configure sync engine
const SYNC_WS_URL = process.env.SYNC_WS_URL || "ws://localhost:7777/sync";
const SYNC_URL = process.env.SYNC_URL || "http://localhost:7777";

const wsAdapter = new BrowserWebSocketClientAdapter(SYNC_WS_URL);
const engine = configureSyncEngine({
  url: SYNC_URL,
  network: [wsAdapter as any as NetworkAdapterInterface],
  storage: new NodeFSStorageAdapter(),
});

// Helper function to handle CORS
const setCorsHeaders = (res: http.ServerResponse) => {
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
  res.setHeader("Access-Control-Allow-Headers", "Content-Type");
};

// Create HTTP server
const server = http.createServer((req, res) => {
  // Set CORS headers for all responses
  setCorsHeaders(res);

  // Handle preflight OPTIONS requests
  if (req.method === "OPTIONS") {
    res.writeHead(204);
    res.end();
    return;
  }

  // Example endpoint
  if (req.method === "GET" && req.url === "/health") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ status: "ok" }));
    return;
  }

  // Main worker endpoint
  if (req.method === "POST" && req.url === "/tonk") {
    let body = "";

    req.on("data", (chunk) => {
      body += chunk.toString();
    });

    req.on("end", async () => {
      try {
        const data = JSON.parse(body);

        // Process the request data
        console.log("Received data:", data);

        // Send success response
        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(
          JSON.stringify({
            success: true,
            message: "Request processed successfully",
          }),
        );
      } catch (error) {
        console.error("Error processing request:", error);
        res.writeHead(400, { "Content-Type": "application/json" });
        res.end(
          JSON.stringify({ success: false, error: "Invalid data format" }),
        );
      }
    });
  } else {
    // Handle other routes
    res.writeHead(404, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ success: false, error: "Not found" }));
  }
});

// Start the server
const PORT = process.env.PORT || 5555;
server.listen(PORT, async () => {
  console.log(`Tonk worker listening on http://localhost:${PORT}/tonk`);

  // Initialize the sync engine
  try {
    await engine.whenReady();
    console.log("Keepsync engine is ready");
  } catch (error) {
    console.error("Error initializing sync engine:", error);
  }

  // Handle graceful shutdown
  const cleanup = () => {
    console.log("Shutting down...");
    process.exit(0);
  };

  process.on("SIGINT", cleanup);
  process.on("SIGTERM", cleanup);
});

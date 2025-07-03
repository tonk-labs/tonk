import * as http from "http";
import dotenv from "dotenv";
import {
  ClaudeApiProvider,
  type LLMRequest,
} from "./services/claudeApiProvider";
import { configureSyncEngine } from "@tonk/keepsync";
import { NetworkAdapterInterface } from "@automerge/automerge-repo";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { NodeFSStorageAdapter } from "@automerge/automerge-repo-storage-nodefs";

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
console.log(`Starting AI worker at ${new Date().toISOString()}`);
console.log(`Node.js version: ${process.version}`);
console.log(`Platform: ${process.platform}`);

/**
 * Configuration for the worker
 */
interface WorkerConfig {
  port: number;
}

// Initialize Claude API provider
const claudeProvider = new ClaudeApiProvider({
  workingDirectory: process.cwd(),
  maxTurns: 10,
  verbose: false,
  outputFormat: "stream-json",
});

/**
 * Start the worker with the given configuration
 */
export async function startWorker(config: WorkerConfig): Promise<http.Server> {
  const { port } = config;

  // Configure sync engine for keepsync
  const SYNC_WS_URL = process.env.SYNC_WS_URL || "ws://localhost:7777/sync";
  const SYNC_URL = process.env.SYNC_URL || "http://localhost:7777";

  const wsAdapter = new BrowserWebSocketClientAdapter(SYNC_WS_URL);
  const engine = configureSyncEngine({
    url: SYNC_URL,
    network: [wsAdapter as any as NetworkAdapterInterface],
    storage: new NodeFSStorageAdapter(),
  });

  await engine.whenReady();
  console.log("✅ Keepsync engine is ready");

  // Helper function to handle CORS
  const setCorsHeaders = (res: http.ServerResponse) => {
    res.setHeader("Access-Control-Allow-Origin", "*");
    res.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
    res.setHeader("Access-Control-Allow-Headers", "Content-Type");
  };

  // Helper function to parse JSON body
  const parseJsonBody = (req: http.IncomingMessage): Promise<any> => {
    return new Promise((resolve, reject) => {
      let body = "";
      req.on("data", (chunk) => {
        body += chunk.toString();
      });
      req.on("end", () => {
        try {
          resolve(JSON.parse(body));
        } catch (error) {
          reject(new Error("Invalid JSON"));
        }
      });
    });
  };

  // Create HTTP server
  const server = http.createServer(async (req, res) => {
    // Set CORS headers for all responses
    setCorsHeaders(res);

    // Handle preflight OPTIONS requests
    if (req.method === "OPTIONS") {
      res.writeHead(204);
      res.end();
      return;
    }

    // Health check endpoint
    if (req.method === "GET" && req.url === "/health") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          status: "ok",
          services: {
            ai: "running",
            keepsync: "running",
            claude: claudeProvider.isConfigured()
              ? "configured"
              : "not configured",
          },
        }),
      );
      return;
    }

    if (req.method === "POST" && req.url === "/api/tonk") {
      try {
        const data = await parseJsonBody(req);

        if (!data.prompt) {
          res.writeHead(400, { "Content-Type": "application/json" });
          res.end(JSON.stringify({ error: "prompt is required" }));
          return;
        }

        const request: LLMRequest = {
          prompt: data.prompt,
          systemPrompt: data.systemPrompt,
          maxTurns: 3,
          allowedTools: data.allowedTools,
          disallowedTools: data.disallowedTools,
          mcpConfig: data.mcpConfig,
          permissionMode: "default",
          verbose: true,
          outputFormat: "text",
        };

        // Handle streaming
        if (data.stream) {
          res.writeHead(200, {
            "Content-Type": "text/plain",
            "Transfer-Encoding": "chunked",
            "Cache-Control": "no-cache",
            Connection: "keep-alive",
          });

          try {
            for await (const chunk of claudeProvider.stream(request)) {
              res.write(chunk);
            }
            res.end();
          } catch (error) {
            res.write(
              `\nError: ${error instanceof Error ? error.message : "Unknown error"}`
            );
            res.end();
          }
          return;
        }

        // Non-streaming response
        const response = await claudeProvider.complete(request);

        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(JSON.stringify(response));
        return;
      } catch (error) {
        console.error("Completion Error:", error);
        res.writeHead(500, { "Content-Type": "application/json" });
        res.end(
          JSON.stringify({
            error:
              error instanceof Error ? error.message : "Internal server error",
          })
        );
        return;
      }
    }

    // 404 for unhandled routes
    res.writeHead(404, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ error: "Not found" }));
  });

  // Start the server
  return new Promise((resolve) => {
    server.listen(port, async () => {
      console.log(`🚀 AI worker listening on http://localhost:${port}`);
      console.log(`📋 Health check: http://localhost:${port}/health`);
      console.log(
        `🧠 Completion endpoint: http://localhost:${port}/api/complete`,
      );
      console.log(`🔗 Main endpoint: http://localhost:${port}/tonk`);

      // Handle graceful shutdown
      const cleanup = async () => {
        console.log("Shutting down...");
        await claudeProvider.cleanup();
        process.exit(0);
      };

      process.on("SIGINT", cleanup);
      process.on("SIGTERM", cleanup);

      resolve(server);
    });
  });
}

// If this file is run directly, start the worker
if (require.main === module) {
  const port = process.env.WORKER_PORT
    ? parseInt(process.env.WORKER_PORT, 10)
    : 5556;
  startWorker({ port })
    .then(() => console.log(`Worker started on port ${port}`))
    .catch((err) => console.error("Failed to start worker:", err));
}

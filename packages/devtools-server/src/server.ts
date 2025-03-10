import express, { Request, Response } from "express";
import { WebSocketServer, WebSocket } from "ws";
import http from "http";
import fs from "fs";
import path from "path";
import process from "process";
import * as pty from "@homebridge/node-pty-prebuilt-multiarch";
import os from "os";

const app = express();
const port = process.env.PORT || 3000;

// Add middleware to parse JSON requests
app.use(express.json());

// Add CORS middleware to allow requests from the extension
app.use((req: Request, res: Response, next) => {
  res.header("Access-Control-Allow-Origin", "*");
  res.header(
    "Access-Control-Allow-Headers",
    "Origin, X-Requested-With, Content-Type, Accept"
  );
  res.header("Access-Control-Allow-Methods", "GET, POST, OPTIONS");

  if (req.method === "OPTIONS") {
    return res.status(200).end();
  }

  next();
});

// Define the path to store the API key
const API_KEY_FILE = path.join(process.cwd(), "./data/api-key.json");

// Ensure the data directory exists
const dataDir = path.dirname(API_KEY_FILE);
if (!fs.existsSync(dataDir)) {
  fs.mkdirSync(dataDir, { recursive: true });
}

// Initialize API key
let apiKey = "";

// Read API key from file if it exists
try {
  if (fs.existsSync(API_KEY_FILE)) {
    const data = fs.readFileSync(API_KEY_FILE, "utf8");
    const config = JSON.parse(data);
    apiKey = config.apiKey || "";
    console.log("API key loaded from file");
  }
} catch (error) {
  console.error("Error reading API key file:", error);
}

// Create HTTP server
const server = http.createServer(app);

// Create WebSocket server
const wss = new WebSocketServer({ server });

// Track active terminal processes
const terminalSessions = new Map<WebSocket, pty.IPty>();

// Default terminal settings
const defaultCols = 80;
const defaultRows = 24;

// Handle WebSocket connections (all connections are treated as terminal sessions)
wss.on("connection", (ws: WebSocket) => {
  console.log("Client connected to terminal session");

  let ptyProcess: pty.IPty | null = null;

  ws.on("message", (message: Buffer | ArrayBuffer | Buffer[]) => {
    try {
      // Parse the incoming message
      const parsedMessage = JSON.parse(message.toString());
      console.log("Received message type:", parsedMessage.type);

      if (parsedMessage.type === "init" && !ptyProcess) {
        // Get shell based on platform
        const shell =
          os.platform() === "win32"
            ? "powershell.exe"
            : parsedMessage.shell || process.env.SHELL || "bash";

        // Get initial dimensions
        const cols = parsedMessage.cols || defaultCols;
        const rows = parsedMessage.rows || defaultRows;

        console.log(
          `Starting terminal with ${shell}, cols: ${cols}, rows: ${rows}`
        );

        // Spawn the PTY process
        ptyProcess = pty.spawn(shell, [], {
          name: "xterm-color",
          cols: cols,
          rows: rows,
          cwd: process.env.HOME || process.cwd(),
          env: process.env as { [key: string]: string },
        });

        // Store the session
        terminalSessions.set(ws, ptyProcess);

        // Send welcome message
        ws.send(
          JSON.stringify({
            type: "output",
            data: `Connected to ${shell} terminal. Type commands to execute them.\r\n`,
          })
        );

        // Handle process data (output)
        ptyProcess.onData((data: string) => {
          ws.send(
            JSON.stringify({
              type: "output",
              data: data,
            })
          );
        });

        // Handle process exit
        ptyProcess.onExit(({ exitCode, signal }) => {
          ws.send(
            JSON.stringify({
              type: "exit",
              data: `\r\nProcess exited with code ${exitCode} (signal: ${signal})\r\n`,
            })
          );
          ptyProcess = null;
          terminalSessions.delete(ws);
        });
      } else if (parsedMessage.type === "command" && ptyProcess) {
        // Write the command to the pty process
        ptyProcess.write(parsedMessage.command);
      } else if (parsedMessage.type === "resize" && ptyProcess) {
        // Handle terminal resize events
        const cols = parsedMessage.cols || defaultCols;
        const rows = parsedMessage.rows || defaultRows;
        console.log(`Resizing terminal to ${cols}x${rows}`);
        ptyProcess.resize(cols, rows);
      } else if (!ptyProcess) {
        // Handle case where terminal process hasn't been initialized
        ws.send(
          JSON.stringify({
            type: "error",
            data: "Error: Terminal session not initialized. Please reconnect.\r\n",
          })
        );
      }
    } catch (error) {
      console.error("Error processing terminal message:", error);
      ws.send(
        JSON.stringify({
          type: "error",
          data: `Error processing message: ${error}\r\n`,
        })
      );
    }
  });

  // Clean up on disconnect
  ws.on("close", () => {
    console.log("Client disconnected from terminal session");
    if (ptyProcess) {
      // Kill the process when the client disconnects
      ptyProcess.kill();
      terminalSessions.delete(ws);
    }
  });
});

app.get("/", (req: Request, res: Response) => {
  res.send("Terminal server for tinyfoot-devtools!");
});

// GET endpoint to retrieve the API key
app.get("/api-key", (req: Request, res: Response) => {
  try {
    res.status(200).json({ apiKey });
  } catch (error) {
    console.error("Error retrieving API key:", error);
    res.status(500).json({ error: "Failed to retrieve API key" });
  }
});

app.post("/api-key", (req: Request, res: Response) => {
  try {
    const { apiKey: newApiKey } = req.body;

    if (!newApiKey) {
      return res.status(400).json({ error: "API key is required" });
    }

    // Update the in-memory API key
    apiKey = newApiKey;

    // Save to file
    fs.writeFileSync(
      API_KEY_FILE,
      JSON.stringify({ apiKey: newApiKey }, null, 2),
      "utf8"
    );

    res.status(200).json({ message: "API key saved successfully" });
  } catch (error) {
    console.error("Error saving API key:", error);
    res.status(500).json({ error: "Failed to save API key" });
  }
});

// Use the HTTP server to listen instead of the Express app directly
server.listen(port, () => {
  console.log(`Terminal server is running at http://localhost:${port}`);
  console.log(`WebSocket terminal server is running on ws://localhost:${port}`);
});

// Graceful shutdown to kill any terminal processes
process.on("SIGINT", () => {
  console.log("Shutting down server and terminal sessions...");

  // Kill all terminal processes
  for (const [ws, proc] of terminalSessions.entries()) {
    proc.kill();
    ws.close();
  }

  // Close the server
  server.close(() => {
    console.log("Server shutdown complete");
    process.exit(0);
  });
});

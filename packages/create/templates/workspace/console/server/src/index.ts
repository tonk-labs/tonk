import express from "express";
import cors from "cors";
import { fileURLToPath } from "url";
import { dirname, join } from "path";
import fs from "fs";
import { spawn } from "child_process";

// Import configuration
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const projectRoot = join(__dirname, "..", "..");

console.log(`📂 API proxy functionality has been removed from this system.`);

const app = express();
const PORT = process.env.PORT || 6080;

// Enable CORS
app.use(cors());

// Parse JSON bodies
app.use(express.json());

// API proxy functionality has been removed
console.log(`🔧 API proxy functionality has been removed from this system.`);

// Health check endpoint
app.get("/api/health", (_req, res) => {
  console.log("🏥 Health check requested");
  res.json({ status: "ok", timestamp: new Date().toISOString() });
});

// Command execution endpoint
app.post(`/api/execute`, (req, res) => {
  const { command, args = [], cwd } = req.body;
  console.log(`🚀 Command execution requested:`);
  console.log(`  Command: ${command}`);
  console.log(`  Args: ${JSON.stringify(args)}`);
  console.log(`  CWD: ${cwd || process.cwd()}`);

  if (!command) {
    console.log("❌ ERROR: No command provided");
    return res.status(400).json({
      error: "Command is required",
      success: false,
    });
  }

  // Validate command for security
  if (typeof command !== "string" || command.trim().length === 0) {
    console.log("❌ ERROR: Invalid command format");
    return res.status(400).json({
      error: "Command must be a non-empty string",
      success: false,
    });
  }

  // Set headers for streaming and send them immediately
  res.writeHead(200, {
    "Content-Type": "text/plain; charset=utf-8",
    "Transfer-Encoding": "chunked",
    "Cache-Control": "no-cache",
    Connection: "keep-alive",
    "X-Accel-Buffering": "no", // Disable nginx buffering if behind proxy
  });

  // Spawn the process
  // Prevent request timeout
  req.setTimeout(0);
  res.setTimeout(0);

  // Keep connection alive
  req.socket.setKeepAlive(true);

  const childProcess = spawn(command, args, {
    cwd: cwd || process.cwd(),
    shell: true,
    stdio: ["pipe", "pipe", "pipe"],
  });

  console.log(`📊 Spawned process with PID: ${childProcess.pid}`);

  let hasOutput = false;

  // Stream stdout
  childProcess.stdout.on("data", (data) => {
    const output = data.toString();
    console.log(
      `📤 STDOUT data received (${output.length} chars): "${output.trim()}"`
    );
    hasOutput = true;
    res.write(`STDOUT: ${data}`);
  });

  // Stream stderr
  childProcess.stderr.on("data", (data) => {
    const output = data.toString();
    console.log(
      `📤 STDERR data received (${output.length} chars): "${output.trim()}"`
    );
    hasOutput = true;
    res.write(`STDERR: ${data}`);
  });

  // Handle process exit
  childProcess.on("close", (code, signal) => {
    console.log(`🏁 Process ${childProcess.pid} closed:`);
    console.log(`  Exit code: ${code}`);
    console.log(`  Signal: ${signal}`);
    console.log(`  Had output: ${hasOutput}`);

    if (signal) {
      res.write(`\nProcess killed with signal: ${signal}\n`);
    } else {
      res.write(`\nProcess exited with code: ${code}\n`);
    }

    if (!hasOutput) {
      console.log("⚠️  No output was received from command");
      res.write("No output from command\n");
    }

    res.end();
  });

  // Handle process errors
  childProcess.on("error", (error: any) => {
    console.log(`❌ Process error for PID ${childProcess.pid}:`);
    console.log(`  Error: ${error.message}`);
    console.log(`  Code: ${error.code || "Unknown"}`);
    console.log(`  Path: ${error.path || "Unknown"}`);
    res.write(`\nError executing command: ${error.message}\n`);
    res.end();
  });

  // Handle client disconnect - but be more careful about when we kill
  let clientDisconnected = false;

  req.on("close", () => {
    clientDisconnected = true;
    console.log(`🔌 Client disconnected for process ${childProcess.pid}`);
    // Give a small delay before killing to avoid race conditions
    setTimeout(() => {
      if (!childProcess.killed) {
        console.log(
          `🛑 Killing process ${childProcess.pid} due to client disconnect`
        );
        childProcess.kill("SIGTERM");
      }
    }, 100);
  });

  req.on("error", (error) => {
    console.log(
      `❌ Request error for process ${childProcess.pid}: ${error.message}`
    );
  });
});

// Global error handler for unhandled routes
app.use("*", (req, res) => {
  console.log(`❓ Unhandled request: ${req.method} ${req.originalUrl}`);
  res.status(404).json({
    error: "Not Found",
    message: "The requested endpoint was not found",
    path: req.originalUrl,
    method: req.method,
  });
});

// Global error handler
app.use(
  (
    err: any,
    req: express.Request,
    res: express.Response,
    _next: express.NextFunction
  ) => {
    console.error(`❌ Unhandled server error:`);
    console.error(`  Path: ${req.method} ${req.originalUrl}`);
    console.error(`  Error: ${err.message}`);
    console.error(`  Stack: ${err.stack}`);

    if (!res.headersSent) {
      res.status(500).json({
        error: "Internal Server Error",
        message: "An unexpected error occurred",
      });
    }
  }
);

// Start the server
const server = app.listen(PORT, () => {
  console.log(`🚀 Server running on port ${PORT}`);
  console.log(`📡 API proxy functionality has been removed from this system.`);
  console.log(
    `🏥 Health check available at: http://localhost:${PORT}/api/health`
  );
  console.log(
    `⚡ Command execution available at: http://localhost:${PORT}/api/execute`
  );
});

// Handle server startup errors
server.on("error", (error: any) => {
  console.error(`❌ Server startup error:`);
  console.error(`  Error: ${error.message}`);
  console.error(`  Code: ${error.code}`);

  if (error.code === "EADDRINUSE") {
    console.error(
      `  Port ${PORT} is already in use. Try a different port or kill the process using it.`
    );
    console.error(`  To find the process: lsof -ti:${PORT}`);
    console.error(`  To kill the process: kill -9 $(lsof -ti:${PORT})`);
  } else if (error.code === "EACCES") {
    console.error(
      `  Permission denied to bind to port ${PORT}. Try using a port >= 1024 or run with sudo.`
    );
  }

  process.exit(1);
});

// Handle process termination gracefully
process.on("SIGTERM", () => {
  console.log("🛑 Received SIGTERM, shutting down gracefully...");
  server.close(() => {
    console.log("✅ Server closed");
    process.exit(0);
  });
});

process.on("SIGINT", () => {
  console.log("🛑 Received SIGINT, shutting down gracefully...");
  server.close(() => {
    console.log("✅ Server closed");
    process.exit(0);
  });
});

// Handle uncaught exceptions
process.on("uncaughtException", (error) => {
  console.error(`💥 Uncaught Exception:`);
  console.error(`  Error: ${error.message}`);
  console.error(`  Stack: ${error.stack}`);
  process.exit(1);
});

// Handle unhandled promise rejections
process.on("unhandledRejection", (reason, promise) => {
  console.error(`💥 Unhandled Rejection at:`, promise);
  console.error(`  Reason:`, reason);
  process.exit(1);
});

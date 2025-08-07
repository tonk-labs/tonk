import cors from "cors";
import express from "express";
import { fileURLToPath } from "url";
import { dirname, join } from "path";
import fs from "fs";
import { promises as fsPromises } from "fs";
import dotenv from "dotenv";
import { ExpressWithRouteTracking } from "./routeTracker.js";
import { UnifiedStreamManager } from "./streamManager.js";

// Import configuration
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const projectRoot = join(__dirname, "..", "..");

// Load environment variables from .env file in project root
dotenv.config({ path: join(projectRoot, ".env") });

// NOTE: if you do not use ExpressWithRouteTracking, the endpoints will break. This is very important.
// You MUST use ExpressWithRouteTracking!
const app = new ExpressWithRouteTracking();
const PORT = process.env.PORT ? parseInt(process.env.PORT, 10) : 6080;

// Enable CORS
app.use(cors());

// Parse JSON bodies
app.use(express.json());

// Initialize stream manager
const streamManager = new UnifiedStreamManager();

// Add ping endpoint for health checks
// WARNING: ALL SERVERS MUST INCLUDE A /ping ENDPOINT FOR HEALTH CHECKS, OTHERWISE THEY WILL FAIL
app.get("/ping", (_req, res) => {
  res.status(200).send("OK");
});

// Add streaming chat endpoint for Tonk Agent
app.post("/api/chat/stream", async (req, res) => {
  try {
    const { message, conversationId, messageHistory, maxSteps, userName } = req.body;

    if (!message || !conversationId) {
      return res.status(400).json({ 
        error: "Missing required fields: message and conversationId" 
      });
    }

    // Set headers for streaming
    res.setHeader('Content-Type', 'text/plain; charset=utf-8');
    res.setHeader('Transfer-Encoding', 'chunked');
    res.setHeader('Cache-Control', 'no-cache');
    res.setHeader('Connection', 'keep-alive');

    // Create and pipe the stream
    const stream = await streamManager.createUnifiedStream({
      message,
      conversationId,
      messageHistory: messageHistory || [],
      maxSteps,
      userName
    });

    const reader = stream.getReader();
    
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        
        res.write(value);
      }
    } finally {
      reader.releaseLock();
      res.end();
    }

  } catch (error) {
    console.error('Streaming error:', error);
    res.status(500).json({ 
      error: error instanceof Error ? error.message : 'Internal server error' 
    });
  }
});

// Add endpoint to list generated widgets
app.get("/api/widgets/list", async (_req, res) => {
  try {
    const widgetsPath = join(projectRoot, 'widgets', 'generated');
    
    // Check if generated directory exists
    try {
      const entries = await fsPromises.readdir(widgetsPath, { withFileTypes: true });
      const widgets = entries
        .filter((entry: any) => entry.isDirectory())
        .map((entry: any) => entry.name);
      
      res.json({ widgets });
    } catch (error) {
      // Directory doesn't exist yet
      res.json({ widgets: [] });
    }
  } catch (error) {
    console.error('Error listing widgets:', error);
    res.status(500).json({ 
      error: error instanceof Error ? error.message : 'Failed to list widgets' 
    });
  }
});

// Check if --routes CLI parameter is provided
const hasRoutesParam = process.argv.includes("--routes");

// Start the server only if --routes parameter is not provided
if (!hasRoutesParam) {
  app.listen(PORT, () => {
    console.log(`Server running on port ${PORT}`);
  });
} else {
  // Output routes in JSON format for nginx generation using tracked routes
  const trackedRoutes = app.getRoutes();
  const routes = trackedRoutes.map((route) => ({
    path: route.path,
    methods:
      route.method === "ALL"
        ? ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"]
        : [route.method],
    ...(route.params && { params: route.params }),
  }));

  // Write routes to file for nginx generation
  const routesFilePath = join(__dirname, "..", "server-routes.json");
  fs.writeFileSync(routesFilePath, JSON.stringify(routes, null, 2));
  console.log(`Routes written to ${routesFilePath}`);
}

import cors from "cors";
import express from "express";
import { fileURLToPath } from "url";
import { dirname, join } from "path";
import fs from "fs";
import dotenv from "dotenv";
import { transform } from "esbuild";
import { ExpressWithRouteTracking } from "./routeTracker.js";
import { UnifiedStreamManager } from "./streamManager.js";
import { configureSyncEngine, ls, readDoc } from '@tonk/keepsync';
import { NetworkAdapterInterface } from "@automerge/automerge-repo";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { NodeFSStorageAdapter } from "@automerge/automerge-repo-storage-nodefs";

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

// Initialize keepsync
let keepsyncReady = false;

async function initializeKeepsync() {
  try {
    const SYNC_WS_URL = process.env.SYNC_WS_URL || "ws://localhost:7777/sync";
    const SYNC_URL = process.env.SYNC_URL || "http://localhost:7777";

    const wsAdapter = new BrowserWebSocketClientAdapter(SYNC_WS_URL);
    const engine = await configureSyncEngine({
      url: SYNC_URL,
      network: [wsAdapter as any as NetworkAdapterInterface],
      storage: new NodeFSStorageAdapter(),
    });

    await engine.whenReady();
    keepsyncReady = true;
    console.log('Keepsync initialized successfully');
  } catch (error) {
    console.error('Failed to initialize keepsync:', error);
  }
}

// Initialize keepsync on startup
initializeKeepsync();

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

// Add endpoint to list generated widgets from keepsync
app.get("/api/widgets/list", async (_req, res) => {
  try {
    if (!keepsyncReady) {
      return res.status(503).json({
        error: 'Keepsync not ready yet'
      });
    }

    // List widgets from keepsync /widgets/generated path
    const generatedPath = '/widgets/generated';

    try {
      const docNode = await ls(generatedPath);

      if (!docNode || !docNode.children) {
        return res.json({ widgets: [] });
      }

      const widgets = docNode.children
        .filter(child => child.type === 'dir')
        .map(child => child.name);

      res.json({ widgets });
    } catch (error) {
      // Directory doesn't exist yet in keepsync
      res.json({ widgets: [] });
    }
  } catch (error) {
    console.error('Error listing widgets from keepsync:', error);
    res.status(500).json({
      error: error instanceof Error ? error.message : 'Failed to list widgets'
    });
  }
});

// Serve widget types as ES module
app.get("/api/widgets/types", (_req, res) => {
  res.setHeader('Content-Type', 'application/javascript');
  res.send(`
export interface WidgetProps {
  id: string;
  x: number;
  y: number;
  width?: number;
  height?: number;
  onMove?: (id: string, deltaX: number, deltaY: number) => void;
  onResize?: (id: string, width: number, height: number) => void;
  selected?: boolean;
  data?: Record<string, any>;
}

export interface WidgetDefinition {
  id: string;
  name: string;
  description: string;
  component: React.ComponentType<WidgetProps>;
  defaultProps?: Partial<WidgetProps>;
  icon?: string;
  category?: string;
}
  `);
});

// Serve BaseWidget component as ES module
app.get("/api/widgets/base-widget", (_req, res) => {
  res.setHeader('Content-Type', 'application/javascript');
  res.send(`
import React from 'https://esm.sh/react@18';

export default function BaseWidget({ children, widget, isSelected, onUpdate }) {
  return React.createElement('div', {
    className: \`widget \${isSelected ? 'selected' : ''}\`,
    style: {
      position: 'absolute',
      left: widget.x,
      top: widget.y,
      width: widget.width,
      height: widget.height,
      border: isSelected ? '2px solid blue' : '1px solid #ccc',
      borderRadius: '4px',
      backgroundColor: 'white',
      boxShadow: '0 2px 4px rgba(0,0,0,0.1)',
      overflow: 'hidden'
    }
  }, children);
}
  `);
});

// Compile and serve widget as complete ES module using ESBuild
app.get("/api/widgets/compiled/:widgetId", async (req, res) => {
  try {
    if (!keepsyncReady) {
      return res.status(503).json({
        error: 'Keepsync not ready yet'
      });
    }

    const { widgetId } = req.params;
    
    // Security: validate widget ID
    if (!widgetId.match(/^[a-zA-Z0-9_-]+$/)) {
      return res.status(400).json({
        error: 'Invalid widget ID'
      });
    }

    // Fetch widget files from keepsync
    const indexDoc = await readDoc<{ content: string; encoding: string; timestamp: number }>(`/widgets/generated/${widgetId}/index.ts`);
    const componentDoc = await readDoc<{ content: string; encoding: string; timestamp: number }>(`/widgets/generated/${widgetId}/component.tsx`);
    
    if (!indexDoc || !componentDoc) {
      return res.status(404).json({
        error: 'Widget files not found'
      });
    }

    // Extract component name from component file - handles multiple patterns
    const functionMatch = componentDoc.content.match(/export\s+default\s+function\s+(\w+)/);
    const constMatch = componentDoc.content.match(/const\s+(\w+):\s*React\.FC.*?export\s+default\s+\1/s);
    const arrowMatch = componentDoc.content.match(/const\s+(\w+)\s*=\s*\([^)]*\)\s*=>/);
    const exportMatch = componentDoc.content.match(/export\s+default\s+(\w+)/);
    
    const componentName = functionMatch?.[1] || constMatch?.[1] || arrowMatch?.[1] || exportMatch?.[1] || 'WidgetComponent';

    // Compile component with ESBuild
    const compiledComponent = await transform(componentDoc.content, {
      loader: 'tsx',
      target: 'es2020',
      format: 'esm',
      jsx: 'automatic'
    });

    // Compile index with ESBuild  
    const compiledIndex = await transform(indexDoc.content, {
      loader: 'ts',
      target: 'es2020',
      format: 'esm'
    });

    // Transform ESBuild's imports to use our CDN and API endpoints
    const transformedComponent = compiledComponent.code
      .replace(/import\s*\{[^}]*\}\s*from\s*['"]react\/jsx-runtime['"];?/g, 'import { jsx, jsxs } from "https://esm.sh/react@18/jsx-runtime";')
      .replace(/import\s*\{[^}]*\}\s*from\s*['"]react['"];?/g, 'import { useState } from "https://esm.sh/react@18";')
      .replace(/import React.*from ['"]react['"];?/g, 'import React from "https://esm.sh/react@18";')
      .replace(/import.*BaseWidget.*from.*['"];?/g, '')
      .replace(/import.*WidgetProps.*from.*['"];?/g, '')
      .replace(/export\s*\{[^}]*\bdefault\b[^}]*\};?/g, '') // Remove export { ... as default }
      .replace(/export\s+default\s+function\s+(\w+)/, 'function $1')
      .replace(/export\s+default\s+/, '');

    const transformedIndex = compiledIndex.code
      .replace(/import React.*from ['"]react['"];?/g, '')
      .replace(/import.*WidgetDefinition.*from.*['"];?/g, '')
      .replace(/import.*from ['"]\.\/component['"];?/g, '')
      .replace(/component:\s*\w+/g, `component: ${componentName}`)
      .replace(/export\s*\{[^}]*\bdefault\b[^}]*\};?/g, '') // Remove export { ... as default }
      .replace(/export\s+default\s+/, '')
      .replace(/var\s+\w+_default\s*=\s*\w+;?/g, ''); // Remove var stdin_default = ...

    // Create the final compiled module that uses the main app's React
    const compiledModule = `
// Get React from the global scope (same instance as main app)
const React = window.React;
const { useState, useEffect, useRef, createElement: jsx, Fragment } = React;

// Helper for JSX
const jsxs = jsx;

// Inline BaseWidget component
function BaseWidget({ children, widget, isSelected, onUpdate }) {
  return jsx('div', {
    className: \`widget \${isSelected ? 'selected' : ''}\`,
    style: {
      position: 'absolute',
      left: widget.x,
      top: widget.y,
      width: widget.width,
      height: widget.height,
      border: isSelected ? '2px solid blue' : '1px solid #ccc',
      borderRadius: '4px',
      backgroundColor: 'white',
      boxShadow: '0 2px 4px rgba(0,0,0,0.1)',
      overflow: 'hidden'
    }
  }, children);
}

// Define types inline for runtime
const WidgetProps = {};
const WidgetDefinition = {};

// Compiled component code (imports removed and JSX transformed)
${transformedComponent
  .replace(/import\s*\{[^}]*\}\s*from\s*['"][^'"]*['"];?\s*/g, '')
  .replace(/\/\* @__PURE__ \*\/ jsx\(/g, 'jsx(')
  .replace(/\/\* @__PURE__ \*\/ jsxs\(/g, 'jsx(')}

// Compiled widget definition
${transformedIndex}

export default widgetDefinition;
    `;

    res.setHeader('Content-Type', 'application/javascript');
    res.setHeader('Cache-Control', 'no-cache'); // Disable caching for development
    res.send(compiledModule);

  } catch (error) {
    console.error('Error compiling widget with ESBuild:', error);
    res.status(500).json({
      error: error instanceof Error ? error.message : 'Failed to compile widget'
    });
  }
});

// Add endpoint to serve widget files from keepsync (keep for debugging)
app.get("/api/widgets/file/:widgetId/:filename", async (req, res) => {
  try {
    if (!keepsyncReady) {
      return res.status(503).json({
        error: 'Keepsync not ready yet'
      });
    }

    const { widgetId, filename } = req.params;

    // Security: validate widget ID and filename
    if (!widgetId.match(/^[a-zA-Z0-9_-]+$/) || !filename.match(/^[a-zA-Z0-9_.-]+$/)) {
      return res.status(400).json({
        error: 'Invalid widget ID or filename'
      });
    }

    const filePath = `/widgets/generated/${widgetId}/${filename}`;

    try {
      const doc = await readDoc<{ content: string; encoding: string; timestamp: number }>(filePath);

      if (!doc) {
        return res.status(404).json({
          error: 'Widget file not found'
        });
      }

      // Set appropriate content type based on file extension
      const ext = filename.split('.').pop()?.toLowerCase();
      let contentType = 'text/plain';
      if (ext === 'ts' || ext === 'tsx') {
        contentType = 'application/typescript';
      } else if (ext === 'js' || ext === 'jsx') {
        contentType = 'application/javascript';
      } else if (ext === 'json') {
        contentType = 'application/json';
      }

      res.setHeader('Content-Type', contentType);
      res.send(doc.content);
    } catch (error) {
      return res.status(404).json({
        error: 'Widget file not found'
      });
    }
  } catch (error) {
    console.error('Error serving widget file from keepsync:', error);
    res.status(500).json({
      error: error instanceof Error ? error.message : 'Failed to serve widget file'
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

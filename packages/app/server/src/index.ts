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
import { WidgetMCPServer } from "./mcpServer.js";

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
    
    // Initialize widget templates after keepsync is ready
    await initializeWidgetTemplates();
  } catch (error) {
    console.error('Failed to initialize keepsync:', error);
  }
}

// Initialize keepsync on startup
initializeKeepsync();

// Initialize widget templates in keepsync
async function initializeWidgetTemplates() {
  if (!keepsyncReady) return;
  
  try {
    const mcpServer = new WidgetMCPServer();
    await mcpServer.start();
    
    // Component template with comprehensive structure and React key best practices
    const componentTemplate = `import React, { useState, useEffect } from 'react';
import { WidgetProps } from '../../index';
import BaseWidget from '../../templates/BaseWidget';

// Define custom props interface extending WidgetProps
interface MyWidgetProps extends WidgetProps {
  // Add custom props specific to your widget
  customProp?: string;
  theme?: 'light' | 'dark';
  size?: 'small' | 'medium' | 'large';
}

// Main widget component - replace MyWidget with your widget name
const MyWidget: React.FC<MyWidgetProps> = ({
  id,
  x,
  y,
  width = 300,  // Default width
  height = 200, // Default height
  selected,
  onMove,
  data,
  // Custom props with defaults
  customProp = 'default',
  theme = 'light',
  size = 'medium',
}) => {
  // State management
  const [state, setState] = useState('initial');
  const [loading, setLoading] = useState(false);

  // Effects for initialization and cleanup
  useEffect(() => {
    // Component initialization logic
    console.log('Widget initialized:', id);
    
    // Cleanup function
    return () => {
      console.log('Widget cleanup:', id);
    };
  }, [id]);

  // Event handlers
  const handleAction = () => {
    setLoading(true);
    // Perform action
    setTimeout(() => {
      setState('updated');
      setLoading(false);
    }, 1000);
  };

  // Computed values
  const themeClasses = theme === 'dark' 
    ? 'bg-gray-800 text-white' 
    : 'bg-white text-gray-800';

  const sizeClasses = {
    small: 'text-sm',
    medium: 'text-base',
    large: 'text-lg'
  }[size];

  // IMPORTANT: When rendering lists, always provide unique keys
  // Example of correct list rendering:
  const renderItems = () => {
    const items = ['item1', 'item2', 'item3'];
    return items.map((item, index) => (
      <div key={\`item-\${index}\`} className="list-item">
        {item}
      </div>
    ));
  };

  return (
    <BaseWidget
      id={id}
      x={x}
      y={y}
      width={width}
      height={height}
      selected={selected}
      onMove={onMove}
      title="My Widget"  // Replace with your widget title
      backgroundColor={theme === 'dark' ? 'bg-gray-800' : 'bg-white'}
      borderColor={theme === 'dark' ? 'border-gray-600' : 'border-gray-200'}
    >
      <div className={\`p-4 flex flex-col h-full \${themeClasses}\`}>
        {/* Header section - single element, no key needed */}
        <div className="flex items-center justify-between mb-4">
          <h3 className={\`font-medium \${sizeClasses}\`}>
            Widget Title
          </h3>
          {loading && (
            <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-current"></div>
          )}
        </div>

        {/* Main content area - single element, no key needed */}
        <div className="flex-1 flex items-center justify-center">
          <div className="text-center">
            <div className={\`font-bold \${sizeClasses} mb-2\`}>
              {customProp}
            </div>
            <div className="text-sm opacity-75 mb-4">
              State: {state}
            </div>
            
            {/* Interactive elements */}
            <button
              onClick={handleAction}
              disabled={loading}
              className="px-4 py-2 bg-blue-500 text-white rounded hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            >
              {loading ? 'Loading...' : 'Action'}
            </button>
          </div>
        </div>

        {/* Footer section - single element, no key needed */}
        <div className="text-xs opacity-50 text-center mt-4">
          Widget ID: {id}
        </div>
      </div>
    </BaseWidget>
  );
};

export default MyWidget;`;

    // Index template
    const indexTemplate = `import React from 'react';
import { WidgetDefinition } from '../../index';
import MyWidgetComponent from './component';

const widgetDefinition: WidgetDefinition = {
  id: 'my-widget',
  name: 'My Widget',
  description: 'A customizable widget template with theme support and interactive elements',
  component: MyWidgetComponent,
  defaultProps: {
    width: 300,
    height: 200,
  },
  icon: '🔧',
  category: 'custom',
};

export default widgetDefinition;`;

    // Write templates to keepsync
    await mcpServer.executeTool('write_widget_file', {
      path: 'templates/component-template.tsx',
      content: componentTemplate
    });

    await mcpServer.executeTool('write_widget_file', {
      path: 'templates/index-template.ts', 
      content: indexTemplate
    });

    // Also ensure the existing widget-template.txt is in keepsync
    await mcpServer.executeTool('write_widget_file', {
      path: 'templates/widget-template.txt',
      content: componentTemplate
    });

    console.log('Widget templates initialized in keepsync');
  } catch (error) {
    console.error('Failed to initialize widget templates:', error);
  }
}

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
const { useState, useEffect, useRef, useCallback, createElement: jsx, Fragment } = React;

// Helper for JSX
const jsxs = jsx;

// Inline BaseWidget component matching the actual BaseWidget interface
function BaseWidget({ 
  id, x, y, width = 200, height = 150, onMove, selected = false, 
  children, className = '', title, backgroundColor = 'bg-white', borderColor = 'border-gray-200' 
}) {
  // Create drag state using useRef
  const dragRef = useRef({ isDragging: false, lastX: 0, lastY: 0 });

  // Handle mouse down on title bar
  const handleMouseDown = useCallback((e) => {
    // Only allow dragging from title bar, not content
    if (e.target.closest('.widget-content')) {
      return;
    }
    
    e.stopPropagation();
    dragRef.current = {
      isDragging: true,
      lastX: e.clientX,
      lastY: e.clientY,
    };
  }, []);

  // Handle mouse move for dragging
  const handleMouseMove = useCallback((e) => {
    if (!dragRef.current.isDragging || !onMove) return;

    const deltaX = e.clientX - dragRef.current.lastX;
    const deltaY = e.clientY - dragRef.current.lastY;

    onMove(id, deltaX, deltaY);

    dragRef.current.lastX = e.clientX;
    dragRef.current.lastY = e.clientY;
  }, [id, onMove]);

  // Handle mouse up to stop dragging
  const handleMouseUp = useCallback(() => {
    dragRef.current.isDragging = false;
  }, []);

  // Set up and clean up event listeners
  useEffect(() => {
    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [handleMouseMove, handleMouseUp]);

  const titleElement = title ? jsx('div', {
    className: 'bg-gray-100 text-gray-800 px-4 py-2 rounded-t-lg cursor-grab active:cursor-grabbing flex items-center justify-between border-b',
    onMouseDown: handleMouseDown,
    key: 'title'
  }, jsx('span', { className: 'font-medium text-sm' }, title)) : null;

  const contentElement = jsx('div', {
    className: 'widget-content flex flex-col h-full',
    style: { height: title ? 'calc(100% - 40px)' : '100%' },
    onWheel: (e) => e.stopPropagation(),
    key: 'content'
  }, children);

  return jsx('div', {
    className: \`absolute \${backgroundColor} rounded-lg shadow-xl border select-none \${
      selected ? 'border-blue-500 border-2' : borderColor
    } \${className}\`,
    style: {
      left: x,
      top: y,
      transform: 'translate(-50%, -50%)',
      width: \`\${width}px\`,
      height: \`\${height}px\`,
      zIndex: 20,
    }
  }, title ? [titleElement, contentElement] : contentElement);
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

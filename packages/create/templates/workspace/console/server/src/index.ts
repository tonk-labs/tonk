import express from "express";
import cors from "cors";
import { createProxyMiddleware } from "http-proxy-middleware";
import { fileURLToPath } from "url";
import { dirname, join } from "path";
import fs from "fs";
import { spawn } from "child_process";

interface ApiService {
  prefix: string; // The route prefix (e.g., "weather")
  baseUrl: string; // The actual API base URL
  requiresAuth?: boolean; // Whether authentication is needed
  authType?: "bearer" | "apikey" | "basic" | "query"; // Authentication type
  authHeaderName?: string; // Header name for auth (e.g., "Authorization" or "X-API-Key")
  authEnvVar?: string; // API key or auth secret
  authQueryParamName?: string; // If using query auth type, the corresponding query param
  isLocal?: boolean; // Whether this is a local service handled by the server itself
}

// Import API services configuration
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const projectRoot = join(__dirname, "..", "..");

// Read API services from the frontend project
const apiServicesPath = join(projectRoot, "src", "services", "apiServices.ts");

let apiServicesContent: string;
let services: ApiService[] = [];

try {
  console.log(`📂 Reading API services configuration from: ${apiServicesPath}`);
  apiServicesContent = fs.readFileSync(apiServicesPath, "utf-8");
  console.log(`✅ Successfully read API services file (${apiServicesContent.length} characters)`);
} catch (error) {
  console.error(`❌ Failed to read API services file: ${apiServicesPath}`);
  console.error(`Error details:`, error);
  process.exit(1);
}

// Extract API services using regex (a simple approach)
const servicesRegex =
  /export const apiServices: ApiService\[\] = \[([\s\S]*?)\];/;
const servicesMatch = apiServicesContent.match(servicesRegex);

if (!servicesMatch) {
  console.error(`❌ Could not parse API services from the frontend project`);
  console.error(`Expected format: export const apiServices: ApiService[] = [...]`);
  console.error(`File content preview:`, apiServicesContent.substring(0, 500));
  process.exit(1);
}

// Parse the services (this is a simplified approach)
const servicesString = servicesMatch[1];

try {
  services = eval(`[${servicesString}]`);
  console.log(`✅ Successfully parsed ${services.length} API services`);
  services.forEach((service, index) => {
    console.log(`  ${index + 1}. ${service.prefix} -> ${service.baseUrl} ${service.requiresAuth ? '(with auth)' : ''}`);
  });
} catch (error) {
  console.error(`❌ Failed to parse API services configuration`);
  console.error(`Services string:`, servicesString);
  console.error(`Parse error:`, error);
  process.exit(1);
}

const app = express();
const PORT = process.env.PORT || 6080;

// Enable CORS
app.use(cors());

// Parse JSON bodies
app.use(express.json());

// Setup API proxies based on the configuration
services.forEach((service: ApiService) => {
  const {
    prefix,
    baseUrl,
    requiresAuth,
    authType,
    authEnvVar,
    authHeaderName,
    authQueryParamName,
    isLocal,
  } = service;

  console.log(`🔧 Setting up proxy for /${prefix} -> ${baseUrl}`);

  // Create proxy middleware for external services
  app.use(
    `/api/${prefix}`,
    createProxyMiddleware({
      target: baseUrl,
      changeOrigin: true,
      pathRewrite: {
        [`^/api/${prefix}`]: "",
      },
      on: {
        proxyReq: (proxyReq, req, _res) => {
          // Add authentication if required
          if (requiresAuth) {
            const authValue = authEnvVar || ""; // In production, this would be process.env[authEnvVar]
            
            if (!authValue && authEnvVar) {
              console.warn(`⚠️  Authentication required for ${prefix} but no value found for ${authEnvVar}`);
            }

            if (authType === "bearer") {
              proxyReq.setHeader(
                authHeaderName || "Authorization",
                `Bearer ${authValue}`,
              );
              console.log(`🔐 Added Bearer auth for ${prefix}`);
            } else if (authType === "apikey") {
              proxyReq.setHeader(authHeaderName || "X-API-Key", authValue);
              console.log(`🔐 Added API key auth for ${prefix}`);
            } else if (authType === "basic") {
              proxyReq.setHeader(
                authHeaderName || "Authorization",
                `Basic ${authValue}`,
              );
              console.log(`🔐 Added Basic auth for ${prefix}`);
            } else if (authType === "query") {
              // For query params, we need to modify the URL
              const url = new URL(proxyReq.path, baseUrl);
              url.searchParams.set(authQueryParamName || "apikey", authValue);
              proxyReq.path = url.pathname + url.search;
              console.log(`🔐 Added query param auth for ${prefix}`);
            }
          }
          // Log the request
          console.log(`🔄 Proxying ${req.method} request to ${baseUrl}${proxyReq.path}`);
        },
        proxyRes: (proxyRes, req, _res) => {
          console.log(`✅ Proxy response for ${req.method} ${req.url}: ${proxyRes.statusCode} ${proxyRes.statusMessage}`);
        },
        error: (err: any, req, res) => {
          console.error(`❌ Proxy error for ${req.method} ${req.url}:`);
          console.error(`  Target: ${baseUrl}`);
          console.error(`  Error: ${err.message}`);
          console.error(`  Code: ${err.code || 'Unknown'}`);
          
          if (res && 'status' in res && 'headersSent' in res && !res.headersSent) {
            (res as express.Response).status(502).json({
              error: 'Proxy Error',
              message: 'Failed to connect to upstream service',
              service: prefix,
              target: baseUrl
            });
          }
        }
      },
    }),
  );
});

// Health check endpoint 
app.get('/api/health', (_req, res) => {
  console.log('🏥 Health check requested');
  res.json({ status: 'ok', timestamp: new Date().toISOString() });
});

// Command execution endpoint
app.post(`/api/execute`, (req, res) => {
  const { command, args = [], cwd } = req.body;
  console.log(`🚀 Command execution requested:`);
  console.log(`  Command: ${command}`);
  console.log(`  Args: ${JSON.stringify(args)}`);
  console.log(`  CWD: ${cwd || process.cwd()}`);

  if (!command) {
    console.log('❌ ERROR: No command provided');
    return res.status(400).json({ 
      error: 'Command is required',
      success: false
    });
  }

  // Validate command for security
  if (typeof command !== 'string' || command.trim().length === 0) {
    console.log('❌ ERROR: Invalid command format');
    return res.status(400).json({ 
      error: 'Command must be a non-empty string',
      success: false
    });
  }

  // Set headers for streaming and send them immediately
  res.writeHead(200, {
    'Content-Type': 'text/plain; charset=utf-8',
    'Transfer-Encoding': 'chunked',
    'Cache-Control': 'no-cache',
    'Connection': 'keep-alive',
    'X-Accel-Buffering': 'no', // Disable nginx buffering if behind proxy
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
    stdio: ['pipe', 'pipe', 'pipe']
  });

  console.log(`📊 Spawned process with PID: ${childProcess.pid}`);

  let hasOutput = false;
  
  // Stream stdout
  childProcess.stdout.on('data', (data) => {
    const output = data.toString();
    console.log(`📤 STDOUT data received (${output.length} chars): "${output.trim()}"`);
    hasOutput = true;
    res.write(`STDOUT: ${data}`);
  });

  // Stream stderr
  childProcess.stderr.on('data', (data) => {
    const output = data.toString();
    console.log(`📤 STDERR data received (${output.length} chars): "${output.trim()}"`);
    hasOutput = true;
    res.write(`STDERR: ${data}`);
  });

  // Handle process exit
  childProcess.on('close', (code, signal) => {
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
      console.log('⚠️  No output was received from command');
      res.write('No output from command\n');
    }
    
    res.end();
  });

  // Handle process errors
  childProcess.on('error', (error: any) => {
    console.log(`❌ Process error for PID ${childProcess.pid}:`);
    console.log(`  Error: ${error.message}`);
    console.log(`  Code: ${error.code || 'Unknown'}`);
    console.log(`  Path: ${error.path || 'Unknown'}`);
    res.write(`\nError executing command: ${error.message}\n`);
    res.end();
  });

  // Handle client disconnect - but be more careful about when we kill
  let clientDisconnected = false;
  
  req.on('close', () => {
    clientDisconnected = true;
    console.log(`🔌 Client disconnected for process ${childProcess.pid}`);
    // Give a small delay before killing to avoid race conditions
    setTimeout(() => {
      if (!childProcess.killed) {
        console.log(`🛑 Killing process ${childProcess.pid} due to client disconnect`);
        childProcess.kill('SIGTERM');
      }
    }, 100);
  });
  
  req.on('error', (error) => {
    console.log(`❌ Request error for process ${childProcess.pid}: ${error.message}`);
  });
});

// Global error handler for unhandled routes
app.use('*', (req, res) => {
  console.log(`❓ Unhandled request: ${req.method} ${req.originalUrl}`);
  res.status(404).json({
    error: 'Not Found',
    message: 'The requested endpoint was not found',
    path: req.originalUrl,
    method: req.method
  });
});

// Global error handler
app.use((err: any, req: express.Request, res: express.Response, _next: express.NextFunction) => {
  console.error(`❌ Unhandled server error:`);
  console.error(`  Path: ${req.method} ${req.originalUrl}`);
  console.error(`  Error: ${err.message}`);
  console.error(`  Stack: ${err.stack}`);
  
  if (!res.headersSent) {
    res.status(500).json({
      error: 'Internal Server Error',
      message: 'An unexpected error occurred'
    });
  }
});

// Start the server
const server = app.listen(PORT, () => {
  console.log(`🚀 API proxy server running on port ${PORT}`);
  console.log(`📡 Proxying the following services:`);
  services.forEach((service: ApiService) => {
    console.log(`  - ${service.prefix} -> ${service.baseUrl}`);
  });
  console.log(`🏥 Health check available at: http://localhost:${PORT}/api/health`);
  console.log(`⚡ Command execution available at: http://localhost:${PORT}/api/execute`);
});

// Handle server startup errors
server.on('error', (error: any) => {
  console.error(`❌ Server startup error:`);
  console.error(`  Error: ${error.message}`);
  console.error(`  Code: ${error.code}`);
  
  if (error.code === 'EADDRINUSE') {
    console.error(`  Port ${PORT} is already in use. Try a different port or kill the process using it.`);
    console.error(`  To find the process: lsof -ti:${PORT}`);
    console.error(`  To kill the process: kill -9 $(lsof -ti:${PORT})`);
  } else if (error.code === 'EACCES') {
    console.error(`  Permission denied to bind to port ${PORT}. Try using a port >= 1024 or run with sudo.`);
  }
  
  process.exit(1);
});

// Handle process termination gracefully
process.on('SIGTERM', () => {
  console.log('🛑 Received SIGTERM, shutting down gracefully...');
  server.close(() => {
    console.log('✅ Server closed');
    process.exit(0);
  });
});

process.on('SIGINT', () => {
  console.log('🛑 Received SIGINT, shutting down gracefully...');
  server.close(() => {
    console.log('✅ Server closed');
    process.exit(0);
  });
});

// Handle uncaught exceptions
process.on('uncaughtException', (error) => {
  console.error(`💥 Uncaught Exception:`);
  console.error(`  Error: ${error.message}`);
  console.error(`  Stack: ${error.stack}`);
  process.exit(1);
});

// Handle unhandled promise rejections
process.on('unhandledRejection', (reason, promise) => {
  console.error(`💥 Unhandled Rejection at:`, promise);
  console.error(`  Reason:`, reason);
  process.exit(1);
});

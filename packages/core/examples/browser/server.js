#!/usr/bin/env node
/**
 * Simple HTTP server with Cross-Origin Isolation headers for WASM testing.
 * This enables SharedArrayBuffer and atomics features required for wasm-bindgen-rayon.
 */

const http = require('http');
const fs = require('fs');
const path = require('path');
const url = require('url');

const MIME_TYPES = {
  '.html': 'text/html',
  '.js': 'application/javascript',
  '.wasm': 'application/wasm',
  '.json': 'application/json',
  '.css': 'text/css',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.gif': 'image/gif',
  '.svg': 'image/svg+xml',
};

function getContentType(filePath) {
  const ext = path.extname(filePath).toLowerCase();
  return MIME_TYPES[ext] || 'text/plain';
}

function createServer(rootDir = '.', port = 8080) {
  const server = http.createServer((req, res) => {
    const parsedUrl = url.parse(req.url);
    let filePath = path.join(rootDir, parsedUrl.pathname);

    // Default to index.html for directory requests
    if (parsedUrl.pathname.endsWith('/')) {
      filePath = path.join(filePath, 'index.html');
    }

    // Security: prevent directory traversal
    const resolvedPath = path.resolve(filePath);
    const resolvedRoot = path.resolve(rootDir);
    if (!resolvedPath.startsWith(resolvedRoot)) {
      console.log(`Forbidden: ${resolvedPath} not under ${resolvedRoot}`);
      res.writeHead(403);
      res.end('Forbidden');
      return;
    }

    console.log(`Serving: ${parsedUrl.pathname} -> ${resolvedPath}`);

    // Set Cross-Origin Isolation headers (required for SharedArrayBuffer)
    res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
    res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
    res.setHeader('Cross-Origin-Resource-Policy', 'cross-origin');

    // CORS headers for development
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type');

    if (req.method === 'OPTIONS') {
      res.writeHead(200);
      res.end();
      return;
    }

    fs.stat(filePath, (err, stats) => {
      if (err || !stats.isFile()) {
        res.writeHead(404);
        res.end('Not Found');
        return;
      }

      const contentType = getContentType(filePath);
      res.setHeader('Content-Type', contentType);

      const stream = fs.createReadStream(filePath);
      stream.pipe(res);

      stream.on('error', () => {
        res.writeHead(500);
        res.end('Server Error');
      });
    });
  });

  return server;
}

function main() {
  const args = process.argv.slice(2);
  const port =
    args.find(arg => arg.startsWith('--port='))?.split('=')[1] || 8080;
  const dir = args.find(arg => arg.startsWith('--dir='))?.split('=')[1] || '.';

  const server = createServer(dir, parseInt(port));

  server.listen(port, '127.0.0.1', () => {
    console.log('Tonk Browser Test Server');
    console.log(`Serving at: http://127.0.0.1:${port}`);
    console.log(`Directory: ${path.resolve(dir)}`);
    console.log(
      `Test Runner: http://127.0.0.1:${port}/examples/browser/test-runner.html`
    );
    console.log('\nPress Ctrl+C to stop the server');
    console.log('='.repeat(60));
  });

  process.on('SIGINT', () => {
    console.log('\n\n🛑 Server stopped');
    server.close();
    process.exit(0);
  });
}

if (require.main === module) {
  main();
}

module.exports = { createServer };

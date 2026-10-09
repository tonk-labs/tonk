import { createServer } from 'node:http';
import { resolve } from 'node:path';
import { startNativeRuntime } from './native.mjs';
import { createTonkHTTP } from './http.mjs';

const [binary, dataDirectory] = process.argv.slice(2);
if (!binary || !dataDirectory || process.argv.length !== 4) {
  console.error('Usage: node dev-server.mjs /path/to/tonk-mcp-runtime /path/to/development-data');
  process.exit(1);
}
const backend = await startNativeRuntime({ binary: resolve(binary), dataDirectory: resolve(dataDirectory) });
const handler = createTonkHTTP(async () => backend);
const server = createServer(async (req, res) => {
  const authority = `127.0.0.1:${server.address().port}`;
  if (req.headers.host !== authority || req.headers.origin || req.headers['sec-fetch-site']) {
    res.writeHead(403).end(); return;
  }
  if (req.url !== '/mcp') { res.writeHead(404).end(); return; }
  const abort = new AbortController();
  res.on('close', () => abort.abort());
  try {
    let size = 0;
    const chunks = [];
    for await (const chunk of req) {
      size += chunk.length;
      if (size > 200000) { res.writeHead(413).end(); return; }
      chunks.push(chunk);
    }
    const request = new Request(`http://${authority}/mcp`, {
      method: req.method, headers: req.headers, signal: abort.signal,
      ...(req.method === 'GET' || req.method === 'HEAD' ? {} : { body: Buffer.concat(chunks) }),
    });
    const response = await handler.fetch(request);
    res.writeHead(response.status, Object.fromEntries(response.headers));
    if (response.body) for await (const chunk of response.body) res.write(chunk);
    res.end();
  } catch {
    if (!res.headersSent) res.writeHead(500);
    res.end();
  }
});
server.listen(Number(process.env.PORT ?? 8787), '127.0.0.1', () => {
  console.error(`Tonk development MCP: http://127.0.0.1:${server.address().port}/mcp`);
});
async function stop() {
  await backend.close();
  await handler.close();
  server.closeAllConnections();
  server.close();
}
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
server.on('error', () => { console.error('Could not start the development HTTP listener.'); void stop(); process.exitCode = 1; });

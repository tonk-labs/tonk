import { createServer } from 'node:http';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createCheckpoint } from './checkpoint.mjs';
import { createDurableHTTP } from './durable-http.mjs';
import { createTonkOAuth } from './oauth.mjs';
import { nativeAccountProvisioner } from './oauth-native.mjs';
import { createTenantBackends } from './tenant-backends.mjs';
import { createAuthenticatedTonkHTTP } from './authenticated-http.mjs';
import { hostedBackend } from './hosted-backend.mjs';

const required = name => {
  const value = process.env[name];
  if (!value) throw new Error(`Missing ${name}`);
  return value;
};
const issuer = required('TONK_MCP_ISSUER');
if (new URL(issuer).origin !== issuer || !issuer.startsWith('https://')) throw new Error('Invalid issuer.');
const subject = required('TONK_MCP_SPACE');
if (!/^did:key:[A-Za-z0-9]+$/.test(subject)) throw new Error('Invalid space DID.');
const binary = required('TONK_MCP_BINARY');
const dataRoot = await mkdtemp(join(tmpdir(), 'tonk-hosted-'));
const checkpoint = await createCheckpoint({ directory: dataRoot });
const oauth = createTonkOAuth({ issuer, clientId: required('TONK_MCP_CLIENT_ID'),
  redirectUris: [required('TONK_MCP_REDIRECT_URI')], linkPage: 'https://tonk.foundation/settings/link',
  provisionAccount: nativeAccountProvisioner({ binary, dataRoot }) });
let backends;
const http = createAuthenticatedTonkHTTP({ oauth, resolveBackend: async principal => {
  backends ??= createTenantBackends({ binary, dataRoot, chooseSubject: () => subject });
  return hostedBackend(await backends.resolve(principal));
} });
const durable = createDurableHTTP({ checkpoint,
  onFailure: detail => console.error(JSON.stringify({ event: 'tonk-runtime-unavailable', ...detail })),
  handle: request => http.fetch(request),
  quiesce: async () => { await backends?.close(); backends = undefined; } });

// This port is private to the Cloudflare Worker/container boundary. Issuer is
// deployment configuration; never construct OAuth URLs from forwarded headers.
const server = createServer(async (req, res) => {
  try {
    if (!req.url?.startsWith('/') || req.url.startsWith('//')) { res.writeHead(400).end(); return; }
    let size = 0;
    const chunks = [];
    for await (const chunk of req) {
      size += chunk.length;
      if (size > 200000) { res.writeHead(413).end(); return; }
      chunks.push(chunk);
    }
    const request = new Request(issuer + req.url, { method: req.method, headers: req.headers,
      ...(req.method === 'GET' || req.method === 'HEAD' ? {} : { body: Buffer.concat(chunks) }) });
    const response = await durable.fetch(request);
    res.writeHead(response.status, Object.fromEntries(response.headers));
    res.end(Buffer.from(await response.arrayBuffer()));
  } catch { if (!res.headersSent) res.writeHead(503); res.end(); }
});
server.requestTimeout = 30000;
server.headersTimeout = 10000;
server.listen(8080, '0.0.0.0');
async function stop() {
  server.close();
  await durable.close();
  await http.close();
  server.closeAllConnections();
}
process.once('SIGTERM', stop);
process.once('SIGINT', stop);

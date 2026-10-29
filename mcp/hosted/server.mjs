// Hosted runtime. `/mcp` is the OAuth endpoint where a human sponsors an agent;
// `/mcp/link` takes per-call agent links, each supplying its own scoped authority.
import { createServer } from 'node:http';
import { Readable } from 'node:stream';
import { readFile } from 'node:fs/promises';
import { createAgentService } from './agent-service.mjs';
import { createSponsorService } from './sponsor-service.mjs';
import { providerOrigin } from './provider.mjs';
const required = (name) => {
  if (!process.env[name]) throw Error('Missing ' + name);
  return process.env[name];
};
const issuer = required('TONK_MCP_ISSUER'),
  binary = required('TONK_AGENT_WORKER_BINARY'),
  provider = providerOrigin(required('TONK_PROVIDER_ORIGIN'));
if (new URL(issuer).origin !== issuer || !issuer.startsWith('https://'))
  throw Error('Invalid issuer');
const build = await readFile(new URL('./build.json', import.meta.url), 'utf8').catch((error) => {
  if (error.code === 'ENOENT') return '{}';
  throw error;
});
const agentLinks = await createAgentService({ issuer, binary, provider });
const sponsor = await createSponsorService({ issuer, provider, agents: agentLinks.agents });
const service = {
  fetch: (request) =>
    new URL(request.url).pathname === '/mcp/link'
      ? agentLinks.fetch(request)
      : sponsor.fetch(request),
  close: () => Promise.all([agentLinks.close(), sponsor.close()]),
};
const server = createServer(async (req, res) => {
  try {
    if (!req.url?.startsWith('/') || req.url.startsWith('//')) {
      res.writeHead(400).end();
      return;
    }
    if (req.url === '/build.json' && req.method === 'GET') {
      res.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' });
      res.end(build);
      return;
    }
    const abort = new AbortController();
    res.on('close', () => abort.abort());
    let size = 0;
    const chunks = [];
    for await (const chunk of req) {
      size += chunk.length;
      if (size > 200000) {
        res.writeHead(413).end();
        return;
      }
      chunks.push(chunk);
    }
    const request = new Request(issuer + req.url, {
      method: req.method,
      headers: req.headers,
      signal: abort.signal,
      ...(['GET', 'HEAD'].includes(req.method) ? {} : { body: Buffer.concat(chunks) }),
    });
    const response = await service.fetch(request);
    res.writeHead(response.status, Object.fromEntries(response.headers));
    if (response.body)
      Readable.fromWeb(response.body)
        .on('error', () => res.destroy())
        .pipe(res);
    else res.end();
  } catch {
    if (!res.headersSent) res.writeHead(503);
    res.end();
  }
});
server.listen(8080, '0.0.0.0');
async function close() {
  server.close();
  await service.close();
  server.closeAllConnections();
}
process.once('SIGTERM', close);
process.once('SIGINT', close);

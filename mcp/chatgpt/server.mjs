// Full-worker hosted runtime. OAuth seeds are checkpointed while closed;
// long-lived worker replicas publish through Tonk's normal remote sync.
import { createServer } from 'node:http';
import { Readable } from 'node:stream';
import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createDurableState } from './durable-state.mjs';
import { createCheckpoint } from './checkpoint.mjs';
import { createTonkOAuth } from './oauth.mjs';
import { createAuthenticatedTonkHTTP } from './authenticated-http.mjs';
import { workerAccountProvisioner, createWorkerTenants } from './worker-tenants.mjs';
import { accountBackend } from '../account-backend.mjs';
import { createWorkerUI } from './ui.mjs';
import { createSpaceSessions } from './space-session.mjs';
import { checkpointedProvisioner } from './provisioning.mjs';
import { providerOrigin } from './provider.mjs';
const required = (name) => {
  if (!process.env[name]) throw Error('Missing ' + name);
  return process.env[name];
};
const issuer = required('TONK_MCP_ISSUER'),
  subject = required('TONK_MCP_SPACE'),
  binary = required('TONK_WORKER_BINARY'),
  provider = providerOrigin(required('TONK_PROVIDER_ORIGIN'));
if (new URL(issuer).origin !== issuer || !issuer.startsWith('https://'))
  throw Error('Invalid issuer');
const build = await readFile(new URL('./build.json', import.meta.url), 'utf8').catch((error) => {
  if (error.code === 'ENOENT') return '{}';
  throw error;
});
const dataRoot = await mkdtemp(join(tmpdir(), 'tonk-worker-seeds-'));
const checkpoint = await createCheckpoint({ directory: dataRoot });
const durable = await createDurableState();
if (durable.state && (durable.state.version !== 1 || durable.state.subject !== subject))
  throw Error('Credential snapshot configuration mismatch');
const provision = workerAccountProvisioner({ binary, dataRoot, provider });
const oauth = createTonkOAuth({
  issuer,
  dynamicRegistration: process.env.TONK_MCP_DCR === 'true',
  clientId: required('TONK_MCP_CLIENT_ID'),
  redirectUris: [required('TONK_MCP_REDIRECT_URI')],
  clients: process.env.TONK_MCP_CLIENTS ? JSON.parse(process.env.TONK_MCP_CLIENTS) : undefined,
  linkPage: provider + '/settings/link',
  saved: durable.state?.oauth,
  onRevoke: (principal) => sessions.revokePrincipal(principal),
  provisionAccount: checkpointedProvisioner({
    provision,
    checkpoint,
    retainedTenants: () => [...oauth.retainedTenants(), ...sessions.retainedTenants()],
  }),
});
const tenants = createWorkerTenants({ binary, dataRoot });
const sessions = createSpaceSessions({
  saved: durable.state?.sessions,
  isPrincipalActive: (principal) => oauth.isPrincipalActive(principal),
  resolveWorker: (principal, selected) => tenants.resolve(principal, selected),
});
const http = createAuthenticatedTonkHTTP({
  oauth,
  registerUI: createWorkerUI({ origin: issuer }),
  resolveBackend: async (principal) => ({
    ...accountBackend({ principal, tenants }),
    async openSpace(space) {
      await tenants.resolve(principal, space);
      return sessions.issue(principal, space);
    },
  }),
});
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
    const response = await durable.run(
      () =>
        new URL(request.url).pathname.startsWith('/space-api/')
          ? sessions.fetch(request)
          : http.fetch(request),
      () => ({ version: 1, subject, oauth: oauth.snapshot(), sessions: sessions.snapshot() }),
    );
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
  sessions.close();
  await http.close();
  await tenants.close();
  server.closeAllConnections();
}
process.once('SIGTERM', close);
process.once('SIGINT', close);

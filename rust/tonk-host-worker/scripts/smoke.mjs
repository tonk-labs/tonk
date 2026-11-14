// node scripts/smoke.mjs <hosted_fixture binary>
//
// An MCP tool call end to end through the Host Worker under `wrangler dev
// --local`: the fixture's access service holds an agent space with one note;
// the Worker, bundled with the space's kernel (scripts/bundle.sh, then
// `worker-build --release`), answers `/mcp`. Queries the note, writes another,
// reads both back, and prints how long each step took. Run from this
// directory.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import assert from 'node:assert/strict';

const fixtureBinary = process.argv[2];
const PORT = Number(process.env.SMOKE_PORT ?? 8796);
const endpoint = `http://127.0.0.1:${PORT}/mcp`;
const children = [];

function stop() {
  for (const child of children) {
    if (child.exitCode === null) process.kill(child.pid, 'SIGTERM');
  }
}
process.on('exit', stop);

/** The first line `child` writes to stdout that `match` accepts. */
function line(child, match) {
  return new Promise((resolve, reject) => {
    const lines = createInterface({ input: child.stdout });
    lines.on('line', (text) => {
      if (match(text)) resolve(text);
    });
    child.on('exit', (code) => reject(new Error(`exited ${code} before the line`)));
  });
}

const fixture = spawn(fixtureBinary, [], { stdio: ['ignore', 'pipe', 'inherit'] });
children.push(fixture);
const { access, token, space } = JSON.parse(await line(fixture, (text) => text.startsWith('{')));

const wrangler = spawn(
  'wrangler',
  [
    'dev', '--local', '--port', String(PORT), '--ip', '127.0.0.1',
    '--var', `ACCESS_SERVICE_URL:${access}`,
    '--var', 'MCP_SESSION_SECRET:smoke',
  ],
  { stdio: ['ignore', 'pipe', 'inherit'] },
);
children.push(wrangler);
await line(wrangler, (text) => text.includes('Ready on'));

let session;
let id = 0;
async function rpc(method, params) {
  const headers = {
    authorization: `Bearer ${token}`,
    'content-type': 'application/json',
    accept: 'application/json, text/event-stream',
  };
  if (session) headers['mcp-session-id'] = session;
  const request = { jsonrpc: '2.0', method, params };
  if (!method.startsWith('notifications/')) request.id = ++id;
  const started = performance.now();
  const response = await fetch(endpoint, { method: 'POST', headers, body: JSON.stringify(request) });
  const ms = performance.now() - started;
  session ??= response.headers.get('mcp-session-id') ?? undefined;
  const text = await response.text();
  return { status: response.status, body: text ? JSON.parse(text) : null, ms, timing: response.headers.get('server-timing') };
}

async function tool(name, args) {
  const answer = await rpc('tools/call', { name, arguments: args });
  assert.equal(answer.status, 200, JSON.stringify(answer.body));
  const result = answer.body.result;
  const text = result.content.map((part) => part.text ?? '').join('\n');
  assert.equal(result.isError, false, text);
  console.log(`${name}: ${Math.round(answer.ms)} ms (${answer.timing})`);
  return text;
}

const initialized = await rpc('initialize', {
  protocolVersion: '2025-06-18',
  capabilities: {},
  clientInfo: { name: 'smoke', version: '0' },
});
assert.equal(initialized.status, 200, JSON.stringify(initialized.body));
assert.ok(session, 'the Worker minted an MCP session');
console.log(`initialize: ${Math.round(initialized.ms)} ms`);
await rpc('notifications/initialized');

const find = 'fixture.note:\n  this: ?note\n  title: ?title\n';
const before = await tool('tonk_query', { space, document: find });
assert.ok(before.includes('written before the agent connected'), before);
const again = await tool('tonk_query', { space, document: find });
assert.ok(again.includes('written before the agent connected'), again);
await tool('tonk_evaluate', {
  space,
  document: 'fixture-note!:\n  this: urn:fixture:smoke\n  title: "written through the Worker"\n',
});
const after = await tool('tonk_query', { space, document: find });
assert.ok(after.includes('written through the Worker'), after);
assert.ok(after.includes('written before the agent connected'), after);
console.log('the Worker answered MCP over the space kernel');
stop();
process.exit(0);

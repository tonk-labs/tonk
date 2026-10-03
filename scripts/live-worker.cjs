// Execute the production Wasm worker, with its space Durable Object, against
// disposable Miniflare R2, D1 and KV: a watch begun over a space's socket must
// be told of a cell write made over HTTP.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { Miniflare, convertV4MiniflareOptions } = require('miniflare');

const SUBPROTOCOL = 'dialog.ucan.v1';

async function main() {
  const [shim, fixtureFile, root, state] = process.argv.slice(2);
  const fixture = JSON.parse(fs.readFileSync(fixtureFile, 'utf8'));
  const options = convertV4MiniflareOptions({
    name: 'live-fixture', resourcePersistencePath: state,
    modulesRoot: path.dirname(path.dirname(shim)),
    modules: [
      { type: 'ESModule', path: shim },
      { type: 'ESModule', path: path.join(path.dirname(shim), '../index.js') },
      { type: 'CompiledWasm', path: path.join(path.dirname(shim), '../index_bg.wasm') },
    ],
    compatibilityDate: '2026-01-01', compatibilityFlags: ['nodejs_compat'],
    r2Buckets: ['BUCKET'],
    kvNamespaces: ['REVOCATIONS_KV', 'SERVABILITY_KV'],
    d1Databases: ['CONTROL', 'INGEST'],
    durableObjects: { LIVE: 'Live' },
    bindings: {
      R2_ACCOUNT_ID: 'local-fixture', R2_BUCKET_NAME: 'live',
      R2_ACCESS_KEY_ID: 'test', R2_SECRET_ACCESS_KEY: 'test',
      SERVICE_SECRET_KEY: '5d'.repeat(32),
    },
  });
  const worker = new Miniflare(options);
  try {
    // Both databases as their migrations leave them: the frames a socket
    // carries are metered into INGEST like requests are.
    async function migrate(binding, directory) {
      const database = await worker.getD1Database(binding);
      const migrations = path.join(root, 'rust/tonk-access-service', directory);
      for (const filename of fs.readdirSync(migrations).filter(name => name.endsWith('.sql')).sort()) {
        const sql = fs.readFileSync(path.join(migrations, filename), 'utf8').replace(/--[^\n]*/g, '');
        const statements = sql.split(';').map(statement => statement.trim()).filter(Boolean);
        await database.batch(statements.map(statement => database.prepare(statement)));
      }
      return database;
    }
    const db = await migrate('CONTROL', 'migrations');
    await migrate('INGEST', 'migrations-ingest');
    // The space is a served customer's, as screening requires.
    await db.prepare("INSERT INTO customer(account,email,verified_at,status,plan,cycle_anchor_at) VALUES(?,?,1,'Active','free@2026-08',1)")
      .bind(fixture.subject, 'live@example.test').run();
    await db.prepare('INSERT INTO subscription(consumer,provider,registered_at) VALUES(?,?,1)')
      .bind(fixture.subject, fixture.subject).run();

    const upgrade = await worker.dispatchFetch(
      `http://localhost/ucan/?sub=${encodeURIComponent(fixture.subject)}`,
      { headers: { Upgrade: 'websocket', 'Sec-WebSocket-Protocol': SUBPROTOCOL } },
    );
    assert.equal(upgrade.status, 101, `upgrade answered ${upgrade.status}`);
    assert.equal(upgrade.headers.get('Sec-WebSocket-Protocol'), SUBPROTOCOL);
    const socket = upgrade.webSocket;
    const frames = [];
    let wake = null;
    socket.addEventListener('message', event => {
      frames.push(Buffer.from(event.data));
      if (wake) wake();
    });
    socket.accept();
    const next = () => new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('no frame arrived')), 15000);
      const take = () => {
        if (frames.length) { clearTimeout(timer); wake = null; resolve(frames.shift()); }
        else wake = take;
      };
      take();
    });

    socket.send(new Uint8Array(fixture.watch));
    const began = await next();
    assert.ok(began.includes(Buffer.from('State')), `the watch was not accepted: ${began.toString()}`);

    const written = await worker.dispatchFetch('http://localhost/ucan/', {
      method: 'POST',
      headers: {
        Authorization: fixture.publish,
        'Content-Type': 'application/octet-stream',
        Accept: 'application/octet-stream',
      },
      body: new Uint8Array(fixture.content),
    });
    assert.equal(written.status, 200, `the write answered ${written.status}: ${await written.text()}`);

    const told = await next();
    assert.ok(told.includes(Buffer.from('State')), `the watch was not told: ${told.toString()}`);
    assert.ok(told.includes(Buffer.from(fixture.content)), 'the watch was told what the cell holds');
    socket.close();
    console.log('live worker: a watch over the socket was told of a write over HTTP');
  } finally { await worker.dispose(); }
}
main().catch(error => { console.error(error); process.exitCode = 1; });

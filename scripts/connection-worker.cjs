// Execute the production Wasm worker against disposable Miniflare D1 and KV.
// Local persistence evidence only: Miniflare does not model global KV caching.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { Miniflare, convertV4MiniflareOptions } = require('miniflare');

async function main() {
  const [shim, fixtureFile, root, state] = process.argv.slice(2);
  const fixture = JSON.parse(fs.readFileSync(fixtureFile, 'utf8'));
  const options = convertV4MiniflareOptions({
    name: 'connection-fixture', resourcePersistencePath: state,
    modulesRoot: path.dirname(path.dirname(shim)),
    modules: [
      { type: 'ESModule', path: shim },
      { type: 'ESModule', path: path.join(path.dirname(shim), '../index.js') },
      { type: 'CompiledWasm', path: path.join(path.dirname(shim), '../index_bg.wasm') },
    ],
    compatibilityDate: '2026-01-01', compatibilityFlags: ['nodejs_compat'],
    kvNamespaces: ['REVOCATIONS_KV'],
    d1Databases: ['CONTROL'],
    bindings: {
      R2_ACCOUNT_ID: 'local-fixture', R2_BUCKET_NAME: 'connections',
      R2_ACCESS_KEY_ID: 'test', R2_SECRET_ACCESS_KEY: 'test',
      SERVICE_SECRET_KEY: '5d'.repeat(32),
    },
  });
  let worker = new Miniflare(options);
  async function post(bytes, status) {
    const response = await worker.dispatchFetch('http://localhost/ucan/', {
      method: 'POST', headers: { 'Content-Type': 'application/cbor' }, body: new Uint8Array(bytes),
    });
    const body = new Uint8Array(await response.arrayBuffer());
    assert.equal(response.status, status, `HTTP ${response.status}: ${Buffer.from(body).toString()}`);
    return body;
  }
  try {
    const db = await worker.getD1Database('CONTROL');
    const migrations = path.join(root, 'rust/tonk-access-service/migrations');
    for (const filename of fs.readdirSync(migrations).filter(name => name.endsWith('.sql')).sort()) {
      const sql = fs.readFileSync(path.join(migrations, filename), 'utf8').replace(/--[^\n]*/g, '');
      const statements = sql.split(';').map(statement => statement.trim()).filter(Boolean);
      await db.batch(statements.map(statement => db.prepare(statement)));
    }
    await db.prepare("INSERT INTO customer(account,email,verified_at,status,plan,cycle_anchor_at) VALUES(?,?,1,'Active','free@2026-08',1)")
      .bind(fixture.subject, 'connection@example.test').run();
    await db.prepare('INSERT INTO subscription(consumer,provider,registered_at) VALUES(?,?,1)')
      .bind(fixture.subject, fixture.subject).run();
    for (const invocation of fixture.invocations) await post(invocation, 200);
    await post(fixture.sibling, 200);
    // This hits the actual authenticated revoke handler and KvRevocationIndex.
    const receipt = JSON.parse(Buffer.from(await post(fixture.revocation, 200)).toString());
    assert.equal(receipt.recorded, true);
    assert.deepEqual(receipt.revoked, fixture.targetBytes);
    assert.equal(receipt.subject, fixture.revoker);
    for (const invocation of fixture.invocations) {
      const reason = JSON.parse(Buffer.from(await post(invocation, 403)).toString());
      assert.equal(reason.kind, 'Revoked');
    }
    await post(fixture.sibling, 200);
    const kv = await worker.getKVNamespace('REVOCATIONS_KV');
    const key = `revoked/${fixture.target}/${fixture.revoker}`;
    assert.equal(await kv.get(key), '', 'empty values must remain present revocations');
    await worker.dispose();
    worker = new Miniflare(options);
    for (const invocation of fixture.invocations) await post(invocation, 403);
    await post(fixture.sibling, 200);
    assert.equal(await (await worker.getKVNamespace('REVOCATIONS_KV')).get(key), '');
    const replay = JSON.parse(Buffer.from(await post(fixture.revocation, 200)).toString());
    assert.equal(replay.recorded, false, 'restart must not forget a recorded revocation');
    console.log('connection worker: standard revocation enforced before and after persisted KV/D1 restart; sibling preserved');
  } finally { await worker.dispose(); }
}
main().catch(error => { console.error(error); process.exitCode = 1; });

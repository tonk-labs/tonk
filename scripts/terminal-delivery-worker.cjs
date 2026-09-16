// Production handler and D1 evidence using disposable local workerd storage.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { Miniflare, convertV4MiniflareOptions } = require('miniflare');

async function main() {
  const [shim, fixtureFile, root, state] = process.argv.slice(2);
  const f = JSON.parse(fs.readFileSync(fixtureFile, 'utf8'));
  const options = convertV4MiniflareOptions({
    name: 'terminal-delivery-fixture', resourcePersistencePath: state,
    modulesRoot: path.dirname(path.dirname(shim)),
    modules: [
      { type: 'ESModule', path: shim },
      { type: 'ESModule', path: path.join(path.dirname(shim), '../index.js') },
      { type: 'CompiledWasm', path: path.join(path.dirname(shim), '../index_bg.wasm') },
    ],
    compatibilityDate: '2026-01-01', compatibilityFlags: ['nodejs_compat'],
    kvNamespaces: ['REVOCATIONS_KV'], d1Databases: ['CONTROL'], r2Buckets: ['BUCKET'],
    bindings: { R2_ACCOUNT_ID: 'local-fixture', R2_BUCKET_NAME: 'connections',
      R2_ACCESS_KEY_ID: 'test', R2_SECRET_ACCESS_KEY: 'test', SERVICE_SECRET_KEY: '5d'.repeat(32) },
  });
  let worker = new Miniflare(options);
  async function post(route, bytes, status) {
    const response = await worker.dispatchFetch(`http://localhost${route}`, {
      method: 'POST', headers: { 'Content-Type': 'application/cbor' }, body: new Uint8Array(bytes),
    });
    const body = new Uint8Array(await response.arrayBuffer());
    assert.equal(response.status, status, `HTTP ${response.status}: ${Buffer.from(body).toString()}`);
    return body;
  }
  const publish = (bytes, status) => post('/connection/delivery', bytes, status);
  const read = (bytes, status) => post('/connection/read', bytes, status);
  try {
    let db = await worker.getD1Database('CONTROL');
    const migrations = path.join(root, 'rust/tonk-access-service/migrations');
    for (const filename of fs.readdirSync(migrations).filter(name => name.endsWith('.sql')).sort()) {
      const sql = fs.readFileSync(path.join(migrations, filename), 'utf8').replace(/--[^\n]*/g, '');
      await db.batch(sql.split(';').map(s => s.trim()).filter(Boolean).map(s => db.prepare(s)));
    }
    await read(f.read, 204);
    await publish(f.approval, 404);
    assert.equal(await db.prepare('SELECT COUNT(*) AS n FROM connection_delivery').first('n'), 0);
    await db.prepare("INSERT INTO customer(account,email,verified_at,status,plan,cycle_anchor_at) VALUES(?,?,1,'Active','free@2026-08',1)")
      .bind(f.account, 'terminal@example.test').run();
    await db.prepare('INSERT INTO subscription(consumer,provider,registered_at) VALUES(?,?,1)')
      .bind(f.account, f.account).run();
    const created = JSON.parse(Buffer.from(await publish(f.approval, 201)).toString());
    assert.equal(created.requestId, f.requestId);
    assert.equal(created.recorded, true);
    await read(f.wrongRead, 204);
    assert.deepEqual(await read(f.read, 200), new Uint8Array(f.approval));
    await publish(f.conflicting, 409);
    await publish(f.expired, 410);
    assert.equal(await db.prepare('SELECT COUNT(*) AS n FROM connection_delivery').first('n'), 1);
    await post('/connection/addition', f.addition, 201);
    await db.prepare("INSERT INTO customer(account,email,verified_at,status,plan,cycle_anchor_at) VALUES(?,?,1,'Active','free@2026-08',1)")
      .bind(f.otherAccount, 'other-terminal@example.test').run();
    await post('/connection/addition', f.forgedAddition, 403);
    const hidden = JSON.parse(Buffer.from(await post('/connection/additions/read', f.wrongAdditionRead, 200)).toString());
    assert.equal(hidden.deliveries.length, 0);
    assert.ok(f.large.length > 2_000_000);
    assert.ok(f.largeAddition.length > 2_000_000);
    const writes = [0,1].map(() => worker.dispatchFetch('http://localhost/connection/delivery', {
      method: 'POST', headers: {'Content-Type':'application/cbor'}, body: new Uint8Array(f.large),
    }));
    const racingRead = await worker.dispatchFetch('http://localhost/connection/read', {
      method: 'POST', headers: {'Content-Type':'application/cbor'}, body: new Uint8Array(f.largeRead),
    });
    const racingBytes=new Uint8Array(await racingRead.arrayBuffer());
    assert.ok([200,204].includes(racingRead.status));
    assert.deepEqual(racingBytes,racingRead.status===200?new Uint8Array(f.large):new Uint8Array());
    const concurrent=await Promise.all(writes);
    assert.deepEqual(concurrent.map(r=>r.status).sort(), [200,201]);
    for (const response of concurrent) await response.arrayBuffer();
    assert.deepEqual(await read(f.largeRead,200),new Uint8Array(f.large));
    await publish(f.largeConflict,409);
    await post('/connection/addition',f.largeAddition,201);
    await post('/connection/addition',f.largeAddition,200);
    await publish(f.keeper,201);
    await post('/connection/addition',f.keeperAddition,201);
    assert.ok(await db.prepare('SELECT MAX(LENGTH(content)) AS n FROM connection_delivery_chunk').first('n') <= 524288);
    assert.ok(await db.prepare('SELECT MAX(LENGTH(content)) AS n FROM connection_addition_chunk').first('n') <= 524288);
    await db.prepare(`CREATE TRIGGER fail_chunk BEFORE INSERT ON connection_delivery_chunk WHEN NEW.request_hash='${f.failedId}' AND NEW.ordinal=1 BEGIN SELECT RAISE(ABORT,'injected chunk failure'); END`).run();
    await publish(f.failed,503);
    assert.equal(await db.prepare('SELECT COUNT(*) AS n FROM connection_delivery WHERE request_hash=?').bind(f.failedId).first('n'),0);
    assert.equal(await db.prepare('SELECT COUNT(*) AS n FROM connection_delivery_chunk WHERE request_hash=?').bind(f.failedId).first('n'),0);
    await db.prepare('DROP TRIGGER fail_chunk').run();
    await worker.dispose();
    worker = new Miniflare(options);
    db = await worker.getD1Database('CONTROL');
    assert.deepEqual(await read(f.read, 200), new Uint8Array(f.approval));
    assert.equal(JSON.parse(Buffer.from(await publish(f.approval, 200)).toString()).recorded, false);
    const additions = JSON.parse(Buffer.from(await post('/connection/additions/read', f.additionRead, 200)).toString());
    assert.equal(additions.deliveries.length, 1);
    assert.deepEqual(Buffer.from(additions.deliveries[0].bytes, 'hex'), Buffer.from(f.addition));
    assert.equal(JSON.parse(Buffer.from(await post('/connection/addition', f.addition, 200)).toString()).recorded, false);
    assert.deepEqual(await read(f.largeRead,200),new Uint8Array(f.large));
    const largePage = JSON.parse(Buffer.from(await post('/connection/additions/read',f.largeAdditionRead,200)).toString());
    assert.deepEqual(Buffer.from(largePage.deliveries[0].bytes,'hex'),Buffer.from(f.largeAddition));
    await post('/ucan/', f.revocation, 200);
    await publish(f.stale, 401);
    await post('/connection/addition', f.staleAddition, 401);
    assert.equal(await db.prepare('SELECT COUNT(*) AS n FROM connection_addition').first('n'), 3);
    assert.equal(JSON.parse(Buffer.from(await post('/connection/addition', f.addition, 200)).toString()).recorded, false);
    assert.equal(await db.prepare('SELECT COUNT(*) AS n FROM connection_delivery').first('n'), 3);
    assert.equal(JSON.parse(Buffer.from(await publish(f.approval, 200)).toString()).recorded, false);
    await post('/ucan/',f.purge,200);
    await read(f.read,204);
    await read(f.largeRead,204);
    assert.equal(JSON.parse(Buffer.from(await post('/connection/additions/read',f.additionRead,200)).toString()).deliveries.length,0);
    assert.deepEqual(await read(f.keeperRead,200),new Uint8Array(f.keeper));
    const keeperPage=JSON.parse(Buffer.from(await post('/connection/additions/read',f.keeperAdditionRead,200)).toString());
    assert.deepEqual(Buffer.from(keeperPage.deliveries[0].bytes,'hex'),Buffer.from(f.keeperAddition));
    const kv=await worker.getKVNamespace('REVOCATIONS_KV');
    assert.notEqual(await kv.get(f.revocationKey),null,'account purge preserves standard revocation facts');
    assert.equal(await db.prepare('SELECT COUNT(*) AS n FROM connection_delivery_chunk WHERE request_hash IN (?,?)').bind(f.requestId,f.largeId).first('n'),0);
    assert.equal(await db.prepare('SELECT COUNT(*) AS n FROM connection_addition_chunk WHERE delivery_id NOT IN(SELECT delivery_id FROM connection_addition)').first('n'),0);
    await post('/ucan/',f.purge,200);
    await publish(f.approval,404);
    assert.equal(await db.prepare('SELECT COUNT(*) AS n FROM connection_delivery WHERE account=?').bind(f.account).first('n'),0);
    console.log('terminal delivery worker: >2MB signed initial/addition atomic chunks survive D1 restart and replay; rollback leaves no partial rows; account purge removes only its mailbox and preserves revocations');
  } finally { await worker.dispose(); }
}
main().catch(error => { console.error(error); process.exitCode = 1; });

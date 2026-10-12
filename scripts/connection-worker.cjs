// Execute the production Wasm worker against disposable Miniflare D1, KV, and R2.
// Local persistence evidence only: Miniflare does not model global KV caching.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { Miniflare, convertV4MiniflareOptions } = require('miniflare');


async function main() {
  const [shim, fixtureFile, root, state] = process.argv.slice(2);
  const fixture = JSON.parse(fs.readFileSync(fixtureFile, 'utf8'));
  const faultShim=path.join(path.dirname(shim),'fault-fixture.mjs');
  fs.writeFileSync(faultShim,`import worker from './${path.basename(shim)}';
export default {fetch(req,env,ctx) {
 // Fetch normalizes an empty stream to Content-Length:0. Remove only that
 // synthetic header to exercise the production empty headerless fallback.
 if(req.headers.get('x-test-no-length') && req.headers.get('Content-Length')==='0') {
   req=new Request(req);req.headers.delete('Content-Length');
 }
 if(req.headers.get('x-test-no-length') && req.headers.has('Content-Length')) throw new Error('fixture unexpectedly supplied Content-Length');
 if(req.headers.get('x-test-index-failure')) {
  const bucket=env.BUCKET;
  env={...env,BUCKET:new Proxy(bucket,{get(target,name) {
   if(name==='head' || name==='list') return ()=>Promise.reject(new Error('injected revocation index failure'));
   const value=Reflect.get(target,name);return typeof value==='function'?value.bind(target):value;
  }})};
 }
 if(req.headers.get('x-test-storage-failure')) {
  const bucket=env.BUCKET;
  env={...env,BUCKET:new Proxy(bucket,{get(target,name) {
   if(name==='get') return (...args)=>String(args[0]).startsWith('__tonk/')?target.get(...args):Promise.reject(new Error('injected storage failure'));
   const value=Reflect.get(target,name);return typeof value==='function'?value.bind(target):value;
  }})};
 }
 return typeof worker === 'function' ? new worker(ctx,env).fetch(req) : worker.fetch(req,env,ctx);
}};`);
  const options = convertV4MiniflareOptions({
    name: 'connection-fixture', resourcePersistencePath: state,
    modulesRoot: path.dirname(path.dirname(shim)),
    modules: [
      { type: 'ESModule', path: faultShim },
      { type: 'ESModule', path: shim },
      { type: 'ESModule', path: path.join(path.dirname(shim), '../index.js') },
      { type: 'CompiledWasm', path: path.join(path.dirname(shim), '../index_bg.wasm') },
    ],
    compatibilityDate: '2026-01-01', compatibilityFlags: ['nodejs_compat'],
    kvNamespaces: ['REVOCATIONS_KV'],
    r2Buckets: ['BUCKET'],
    d1Databases: ['CONTROL','INGEST'],
    bindings: {
      R2_ACCOUNT_ID: 'local-fixture', R2_BUCKET_NAME: 'connections',
      R2_ACCESS_KEY_ID: 'test', R2_SECRET_ACCESS_KEY: 'test',
      SERVICE_SECRET_KEY: '5d'.repeat(32),
    },
  });
  let worker = new Miniflare(options);
  function directRequest(bytes) {
    return {method:'POST',url:'http://localhost/ucan/',headers:[['Authorization','UCAN C'+Buffer.from(bytes).toString('base64url')],['Accept','application/octet-stream']]};
  }
  async function post(bytes, status, legacy=false) {
    const response = await worker.dispatchFetch('http://localhost/ucan/', {
      ...(legacy || bytes===fixture.revocation ? {method:'POST',headers:{'Content-Type':'application/cbor'},body:new Uint8Array(bytes)} : {method:'POST',headers:directRequest(bytes).headers}),
    });
    const body = new Uint8Array(await response.arrayBuffer());
    assert.equal(response.status, status, `HTTP ${response.status}: ${Buffer.from(body).toString()}`);
    return body;
  }
  let issued = [], mutations = [], sibling;
  async function redeem(permit, status, body, extraHeaders={}) {
    const headers = new Headers(permit.headers);
    for (const [key,value] of Object.entries(extraHeaders)) headers.set(key,value);
    if (body !== undefined) headers.set('Content-Length', String(Buffer.byteLength(body)));
    const response = await worker.dispatchFetch(permit.url, {method:permit.method,headers,...(body !== undefined ? {body} : {})});
    const bytes = Buffer.from(await response.arrayBuffer());
    assert.equal(response.status,status,`${permit.method} ${permit.url}: ${response.status} ${bytes}`);
    return bytes;
  }
  async function checkRevokedPermits() {
    for (const permit of issued) await redeem(permit,403);
    for (const [permit,body] of mutations) await redeem(permit,403,body);
    assert.deepEqual(await redeem(sibling,200),Buffer.from(fixture.content));
    assert.ok(await (await worker.getR2Bucket('BUCKET')).head(fixture.subject+'/branch/main/empty'),'revoked DELETE leaves cell intact');
  }
  try {
    const db = await worker.getD1Database('CONTROL');
    const migrations = path.join(root, 'rust/tonk-access-service/migrations');
    for (const filename of fs.readdirSync(migrations).filter(name => name.endsWith('.sql')).sort()) {
      const sql = fs.readFileSync(path.join(migrations, filename), 'utf8').replace(/--[^\n]*/g, '');
      const statements = sql.split(';').map(statement => statement.trim()).filter(Boolean);
      await db.batch(statements.map(statement => db.prepare(statement)));
    }
    const ingest=await worker.getD1Database('INGEST');
    const ingestSql=fs.readFileSync(path.join(root,'rust/tonk-access-service/migrations-ingest/0001_ingest.sql'),'utf8').replace(/--[^\n]*/g,'');
    await ingest.batch(ingestSql.split(';').map(s=>s.trim()).filter(Boolean).map(s=>ingest.prepare(s)));
    await db.prepare("INSERT INTO customer(account,email,verified_at,status,plan,cycle_anchor_at) VALUES(?,?,1,'Active','free@2026-08',1)")
      .bind(fixture.subject, 'connection@example.test').run();
    await db.prepare('INSERT INTO subscription(consumer,provider,registered_at) VALUES(?,?,1)')
      .bind(fixture.subject, fixture.subject).run();
    for (const invocation of fixture.invocations) issued.push(directRequest(invocation));
    sibling=directRequest(fixture.sibling);
    const legacy=JSON.parse(Buffer.from(await post(fixture.invocations[0],426,true)).toString());
    assert.equal(legacy.kind,'Unclassified');assert.equal(legacy.code,'UpgradeRequired');
    assert.match(legacy.detail,/npm install -g @tonk\/cli@0\.6\.17/);assert.match(legacy.detail,/not a working upgrade/);
    assert.ok(issued.every(request=>request.headers.every(([name])=>!name.toLowerCase().includes('version'))),'current UI/direct requests omit version metadata');
    const spoof=await worker.dispatchFetch('http://localhost/ucan/',{method:'POST',headers:{'Content-Type':'application/cbor','X-Tonk-Version':'999.0.0'},body:new Uint8Array(fixture.invocations[0])});
    assert.equal(spoof.status,426,'a claimed new version cannot restore obsolete protocol');
    const bucket = await worker.getR2Bucket('BUCKET');
    await bucket.put(fixture.objectKey,Buffer.from(fixture.content));
    for (const permit of issued) assert.deepEqual(await redeem(permit,200),Buffer.from(fixture.content));
    for (const method of ['GET','PUT','DELETE']) {
      const response=await worker.dispatchFetch('http://localhost/object/old?permit=old&signature=old',{method});
      assert.equal(response.status,401);
      assert.equal((await response.json()).kind,'ReauthorizationRequired');
    }
    const content=Buffer.from(fixture.content);
    for (const item of fixture.writes) {
      const before=await ingest.prepare('SELECT COALESCE(SUM(bytes),0) AS bytes FROM invocation').first();
      const permit=directRequest(item.invocation);
      await redeem(permit,200,content);
      const after=await ingest.prepare('SELECT COALESCE(SUM(bytes),0) AS bytes FROM invocation').first();
      assert.equal(after.bytes-before.bytes,content.length,'one transfer is charged once');
      const rows=await ingest.prepare("SELECT COUNT(*) AS n FROM invocation WHERE outcome='ok'").first();
      if(item.resource!=='memory/cell') await redeem(permit,400,Buffer.alloc(content.length));
      const rejected=await ingest.prepare("SELECT COUNT(*) AS n FROM invocation WHERE outcome='ok'").first();
      assert.equal(rejected.n,rows.n,'bad digest is not a successful upload');
      if(item.resource==='memory/cell') await redeem(permit,412,content);
      mutations.push([permit,content]);
    }
    const deletePermit=directRequest(fixture.delete);
    const deleteKey=fixture.subject+'/branch/main/empty';
    await bucket.put(deleteKey,'');
    await redeem(deletePermit,204);
    await bucket.put(deleteKey,'');
    mutations.push([deletePermit,undefined]);
    const okBeforeFailure=await ingest.prepare("SELECT COUNT(*) AS n FROM invocation WHERE outcome='ok'").first();
    await redeem(issued[0],503,undefined,{'Cache-Control':'no-store','x-test-storage-failure':'true'});
    const okAfterFailure=await ingest.prepare("SELECT COUNT(*) AS n FROM invocation WHERE outcome='ok'").first();
    assert.equal(okAfterFailure.n,okBeforeFailure.n,'storage failure is not successful usage');
    // ReadableStream deliberately has no Content-Length. The wrapper asserts
    // that the production handler sees no header (not an inferred fetch size).
    async function headerless(permit,body,status) {
      const headers=new Headers(permit.headers); headers.delete('Content-Length');
      headers.set('x-test-no-length','true');
      const stream=new ReadableStream({start(controller) {controller.enqueue(body);controller.close();}});
      const response=await worker.dispatchFetch(permit.url,{method:permit.method,headers,body:stream,duplex:'half'});
      const text=await response.text();assert.equal(response.status,status,text);
    }
    const blobPermit=directRequest(fixture.writes.find(w=>w.resource==='archive/blob').invocation);
    const beforeHeaderless=await ingest.prepare('SELECT COALESCE(SUM(bytes),0) AS bytes FROM invocation').first();
    await headerless(blobPermit,content,200);
    const afterHeaderless=await ingest.prepare('SELECT COALESCE(SUM(bytes),0) AS bytes FROM invocation').first();
    assert.equal(afterHeaderless.bytes-beforeHeaderless.bytes,content.length,'signed size meters headerless upload');
    await headerless(blobPermit,content.subarray(0,content.length-1),400);
    await headerless(blobPermit,Buffer.concat([content,Buffer.from([1])]),400);
    await headerless(blobPermit,Buffer.alloc(content.length),400);
    assert.deepEqual(await redeem(directRequest(fixture.ranges[2]),200,undefined,{'Cache-Control':'no-store'}),content,'invalid headerless uploads preserve prior object');
    for(const [invocation,key] of [
      [fixture.writes.find(w=>w.resource==='archive/block').invocation,fixture.objectKey],
      [fixture.headerlessCell,fixture.subject+'/branch/main/headerless']
    ]) {
      const permit=directRequest(invocation);
      const before=await ingest.prepare('SELECT COALESCE(SUM(bytes),0) AS bytes FROM invocation').first();
      await headerless(permit,content,200);
      const after=await ingest.prepare('SELECT COALESCE(SUM(bytes),0) AS bytes FROM invocation').first();
      assert.equal(after.bytes-before.bytes,content.length,'collected transfer metered once');
      const rows=await ingest.prepare("SELECT COUNT(*) AS n FROM invocation WHERE outcome='ok'").first();
      for(const invalid of [Buffer.alloc(content.length),content.subarray(0,content.length-1)]) {
        await headerless(permit,invalid,400);
        assert.deepEqual(Buffer.from(await (await bucket.get(key)).arrayBuffer()),content,'bad collected payload preserves existing object');
      }
      await headerless(permit,Buffer.alloc(0),411);
      assert.equal((await ingest.prepare("SELECT COUNT(*) AS n FROM invocation WHERE outcome='ok'").first()).n,rows.n,'invalid collected transfers not metered as success');
    }
    const firstRange=directRequest(fixture.ranges[0]);
    assert.deepEqual(await redeem(firstRange,206,undefined,{Range:'bytes=0-2'}),content.subarray(0,3));
    assert.deepEqual(await redeem(firstRange,206,undefined,{Range:'bytes=3-5'}),content.subarray(0,3),'HTTP Range never changes signed arguments');
    await redeem(firstRange,206);
    // Modern direct requests bypass the legacy redeem/cache entirely.
    const secondRange=directRequest(fixture.ranges[1]);
    const direct=permit=>({...permit,url:permit.url.split('?')[0]});
    const [directFirst,directSecond]=await Promise.all([
      redeem(direct(firstRange),206),redeem(direct(secondRange),206)
    ]);
    assert.deepEqual(directFirst,content.subarray(0,3));
    assert.deepEqual(directSecond,content.subarray(3,6));

    await redeem(directRequest(fixture.empty),200,Buffer.alloc(0));
    await redeem(directRequest(fixture.badEmpty),400,Buffer.alloc(0));
    const large=directRequest(fixture.large);
    await redeem(large,200,Buffer.alloc(33*1024*1024,7));
    await bucket.put(fixture.secondKey,Buffer.from(fixture.secondContent));
    if(process.env.TONK_RELEASED_CLI_FIXTURES) {
      const {spawn}=require('node:child_process');
      const endpoint=new URL('/ucan/',await worker.ready);endpoint.hostname='host.docker.internal';
      const probePath=path.join(root,'scripts/cli-upgrade-probe.cjs');
      await new Promise((resolve,reject)=>{
        const child=spawn('docker',['run','--rm','--platform','linux/amd64','-v',process.env.TONK_RELEASED_CLI_FIXTURES+':/probe:ro','-v',probePath+':/cli-upgrade-probe.cjs:ro','node:24-trixie-slim','node','/cli-upgrade-probe.cjs','015',endpoint.toString()],{stdio:'inherit'});
        child.on('error',reject);child.on('exit',code=>code===0?resolve():reject(new Error('actual old CLI upgrade probe failed: '+code)));
      });
    }
    // Mutable ticket discovery stays on CBOR, outside object response caching.
    await bucket.put(fixture.ticketKey, content);
    for (const claim of [fixture.claim,fixture.selfClaim,fixture.siblingClaim]) {
      assert.deepEqual(Buffer.from(await post(claim,200,true)),content);
    }
    const indexFailure=await worker.dispatchFetch('http://localhost/ucan/',{method:'POST',headers:{'Content-Type':'application/cbor','x-test-index-failure':'1'},body:new Uint8Array(fixture.claim)});
    assert.equal(indexFailure.status,503,'claim index errors fail closed as unavailable');
    await indexFailure.arrayBuffer();
    // Revocation accepts registered consumers even without a serving provider.
    // Keep this holder unprovisioned: claim admission screens only the space.
    await db.prepare('INSERT INTO subscription(consumer,provider,registered_at) VALUES(?,NULL,1)')
      .bind(fixture.claimRevoker).run();
    await post(fixture.claimRevocation,200,true);
    const claimKvKey=`revoked/${fixture.claimTarget}/${fixture.claimRevoker}`;
    const claimStrongKey=`__tonk/revocations/${fixture.claimTarget}/${fixture.claimRevoker}`;
    assert.ok(await bucket.head(claimStrongKey));
    await (await worker.getKVNamespace('REVOCATIONS_KV')).delete(claimKvKey);
    async function checkClaims() {
      const denied=JSON.parse(Buffer.from(await post(fixture.claim,403,true)).toString());
      assert.equal(denied.kind,'Revoked');
      for (const claim of [fixture.selfClaim,fixture.siblingClaim]) assert.deepEqual(Buffer.from(await post(claim,200,true)),content);
    }
    await checkClaims();
    console.log('Worker direct execution: transfer, invalid payload preservation, range, CAS, DELETE, 33MiB stream, metering and legacy upgrade gate passed');
    // This hits the actual authenticated revoke handler and the R2/KV mirrored index.
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
    const strongKey = `__tonk/revocations/${fixture.target}/${fixture.revoker}`;
    assert.ok(await (await worker.getR2Bucket('BUCKET')).head(strongKey), 'R2 fact must precede acknowledgement');
    // Model an absent legacy mirror in this disposable fixture: the actual
    // Worker must still reject from R2, not merely observe Miniflare's fresh KV.
    await kv.delete(key);
    for (const invocation of fixture.invocations) await post(invocation, 403);
    await checkRevokedPermits();
    await post(fixture.sibling, 200);
    await kv.put(key, '');
    await worker.dispose();
    worker = new Miniflare(options);
    for (const invocation of fixture.invocations) await post(invocation, 403);
    await post(fixture.sibling, 200);
    assert.equal(await (await worker.getKVNamespace('REVOCATIONS_KV')).get(key), '');
    assert.ok(await (await worker.getR2Bucket('BUCKET')).head(strongKey), 'R2 revocation must survive restart');
    await checkRevokedPermits();
    await checkClaims();
    assert.equal(await (await worker.getKVNamespace('REVOCATIONS_KV')).get(claimKvKey),null);
    await (await worker.getR2Bucket('BUCKET')).delete(fixture.ticketKey);
    await post(fixture.selfClaim,404,true);
    await post(fixture.siblingClaim,404,true);
    const replay = JSON.parse(Buffer.from(await post(fixture.revocation, 200)).toString());
    assert.equal(replay.recorded, false, 'restart must not forget a recorded revocation');
    console.log('connection worker: standard revocation enforced before and after persisted R2/KV/D1 restart; sibling preserved');
  } finally { await worker.dispose(); }
}
main().catch(error => { console.error(error); process.exitCode = 1; });

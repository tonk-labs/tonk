import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, mkdir, writeFile, readFile, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createCheckpoint } from './checkpoint.mjs';
import { createDurableHTTP } from './durable-http.mjs';
import { startNativeRuntime } from './native.mjs';

function storage() {
  let bytes, version = 0;
  return async (_url, options = {}) => {
    if (options.method !== 'PUT') return bytes === undefined ? new Response(null, { status: 404 }) :
      new Response(bytes, { headers: { etag: `"${version}"` } });
    if ((version && options.headers['if-match'] !== `"${version}"`) ||
      (!version && options.headers['if-none-match'] !== '*')) return new Response(null, { status: 412 });
    bytes = options.body; version++;
    return new Response(null, { headers: { etag: `"${version}"` } });
  };
}

test('checkpoint restores private state and rejects stale replacement, links and traversal', async () => {
  const root = await mkdtemp(join(tmpdir(), 'tonk-checkpoint-'));
  try {
    const a = join(root, 'a'), b = join(root, 'b'), c = join(root, 'c');
    for (const dir of [a, b, c]) await mkdir(dir, { mode: 0o700 });
    const fetch = storage();
    const first = await createCheckpoint({ directory: a, fetch });
    await writeFile(join(a, 'identity'), Buffer.from([0, 1, 255]));
    await first.save();
    const second = await createCheckpoint({ directory: b, fetch });
    assert.deepEqual(await readFile(join(b, 'identity')), Buffer.from([0, 1, 255]));
    await writeFile(join(a, 'identity'), 'new generation'); await first.save();
    await writeFile(join(b, 'identity'), 'stale generation');
    await assert.rejects(second.save(), /Checkpoint write failed/);
    await symlink(join(a, 'identity'), join(a, 'link'));
    await assert.rejects(first.save(), /refuses links/);
    await assert.rejects(createCheckpoint({ directory: c, fetch: async () =>
      Response.json({ version: 1, files: [{ path: '../escape', data: '' }] }, { headers: { etag: 'x' } }) }), /Invalid checkpoint file/);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('successful HTTP responses wait for checkpoint; failed persistence stops queued writes', async () => {
  let release, writes = 0, saves = 0;
  const failures = [];
  const gate = new Promise(resolve => { release = resolve; });
  const service = createDurableHTTP({ handle: async () => { writes++; return new Response('accepted'); },
    quiesce: async () => {}, onFailure: detail => failures.push(detail),
    checkpoint: { save: async () => { saves++; await gate; throw new Error('private credential or path'); } } });
  const first = service.fetch(new Request('https://example.test/mcp'));
  const second = service.fetch(new Request('https://example.test/mcp'));
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(writes, 1); assert.equal(saves, 1);
  release();
  assert.equal((await first).status, 503); assert.equal((await second).status, 503);
  assert.equal(writes, 1);
  assert.deepEqual(failures, [{ stage: 'checkpoint', reason: 'unavailable' }]);
  await service.close();
});

test('native accepted edit survives restoration into a completely fresh runtime directory', {
  skip: process.env.TONK_MCP_RUNTIME ? false : 'Requires native runtime binary.',
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'tonk-native-checkpoint-'));
  let runtime;
  try {
    const first = join(root, 'first'), replacement = join(root, 'replacement');
    await mkdir(first, { mode: 0o700 }); await mkdir(replacement, { mode: 0o700 });
    const fetch = storage();
    const checkpoint = await createCheckpoint({ directory: first, fetch });
    runtime = await startNativeRuntime({ binary: process.env.TONK_MCP_RUNTIME, dataDirectory: join(first, 'tenant') });
    const document = 'concept!: &book\n  description: "A test book"\n  with:\n    title:\n      description: "Title"\n      the: example.test/title\n      as: text\n      cardinality: one\n\nbook!:\n  this: urn:test:book\n  title: "Checkpoint book"\n';
    const preview = await runtime.call('tonk_preview', { document });
    await runtime.call('tonk_apply', { document, expectedRevision: preview.revision });
    const libraryPreview = await runtime.call('tonk_install_library', { component: 'notebook' });
    await runtime.call('tonk_install_library', { component: 'notebook', expectedRevision: libraryPreview.revision });
    const before = await runtime.call('tonk_space_info', {});
    const query = await runtime.call('tonk_query', { document: 'book:\n' });
    await runtime.close(); runtime = undefined;
    await checkpoint.save();
    await createCheckpoint({ directory: replacement, fetch });
    runtime = await startNativeRuntime({ binary: process.env.TONK_MCP_RUNTIME, dataDirectory: join(replacement, 'tenant') });
    assert.deepEqual(await runtime.call('tonk_space_info', {}), before);
    assert.equal((await runtime.call('tonk_install_library', { component: 'notebook' })).alreadyInstalled, true);
    assert.deepEqual(await runtime.call('tonk_query', { document: 'book:\n' }), query);
  } finally { await runtime?.close(); await rm(root, { recursive: true, force: true }); }
});

test('checkpoint saves and restores replicas beyond the 64 MiB JSON ceiling using bounded compressed storage', async () => {
  const root = await mkdtemp(join(tmpdir(), 'tonk-checkpoint-capacity-'));
  try {
    const first = join(root, 'first'), restored = join(root, 'restored');
    await mkdir(first); await mkdir(restored);
    const fetch = storage();
    const checkpoint = await createCheckpoint({ directory: first, fetch });
    const bytes = Buffer.alloc(51 * 1024 * 1024, 7); // Base64 is over 64 MiB.
    await writeFile(join(first, 'synthetic-replica'), bytes);
    await checkpoint.save();
    await createCheckpoint({ directory: restored, fetch });
    assert.deepEqual(await readFile(join(restored, 'synthetic-replica')), bytes);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('selected recovery roots omit abandoned replicas without deleting local data', async () => {
  const root = await mkdtemp(join(tmpdir(), 'tonk-checkpoint-retention-'));
  try {
    const source=join(root,'source'),restored=join(root,'restored');
    await mkdir(source);await mkdir(restored);
    const fetch=storage(),checkpoint=await createCheckpoint({directory:source,fetch});
    for(const name of ['active','pending','expired']){
      await mkdir(join(source,name));await writeFile(join(source,name,'identity'),name);
    }
    await checkpoint.save();
    // Old abandoned replicas must not exhaust the next connection's file budget.
    for(let i=0;i<4096;i++)await writeFile(join(source,'expired',String(i)),'x');
    await assert.rejects(checkpoint.save(),error=>error.checkpointReason==='file-limit');
    await checkpoint.save({roots:new Set(['active','pending'])});
    await createCheckpoint({directory:restored,fetch});
    assert.equal(await readFile(join(restored,'active','identity'),'utf8'),'active');
    assert.equal(await readFile(join(restored,'pending','identity'),'utf8'),'pending');
    await assert.rejects(readFile(join(restored,'expired','identity')), {code:'ENOENT'});
    assert.equal(await readFile(join(source,'expired','identity'),'utf8'),'expired');
  } finally {await rm(root,{recursive:true,force:true});}
});

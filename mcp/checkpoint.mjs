import { readdir, readFile, writeFile, mkdir, lstat } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { createHash } from 'node:crypto';

import { gzipSync, gunzipSync } from 'node:zlib';
import { checkpointLimit, checkpointExpandedLimit } from './checkpoint-limits.mjs';
export { checkpointLimit } from './checkpoint-limits.mjs';
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const validPath = path => typeof path === 'string' && path.length < 1024 &&
  path.split('/').every(part => part && part !== '.' && part !== '..' && !/[\\\x00-\x1f]/.test(part));

// The caller serializes requests and closes every native process before save.
// One private R2 object holds this bounded test deployment's complete state.
// Conditional writes prevent a stale container replacing a newer checkpoint.
export async function createCheckpoint({ directory, fetch: request = globalThis.fetch,
  endpoint = 'http://checkpoint.internal/state' }) {
  if ((await readdir(directory)).length) throw new Error('Restore requires an empty private directory.');
  let etag, previous;
  const response = await request(endpoint, { signal: AbortSignal.timeout(30000), redirect: 'error' });
  if (response.status !== 404) {
    if (!response.ok || !response.headers.get('etag')) throw new Error('Could not restore checkpoint.');
    const reader = response.body.getReader();
    const chunks = [];
    let size = 0;
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        size += value.length;
        if (size > checkpointLimit) { await reader.cancel(); throw new Error('Checkpoint exceeds limit.'); }
        chunks.push(value);
      }
    } finally { reader.releaseLock(); }
    const wire = Buffer.concat(chunks);
    const bytes = wire[0] === 0x1f && wire[1] === 0x8b
      ? gunzipSync(wire, { maxOutputLength: checkpointExpandedLimit }) : wire;
    const saved = JSON.parse(bytes);
    if (saved.version !== 1 || !Array.isArray(saved.files) || saved.files.length > 4096) throw new Error('Invalid checkpoint.');
    const seen = new Set();
    for (const file of saved.files) {
      if (!validPath(file.path) || seen.has(file.path) || typeof file.data !== 'string' ||
        Buffer.from(file.data, 'base64').toString('base64') !== file.data) throw new Error('Invalid checkpoint file.');
      seen.add(file.path);
    }
    for (const file of saved.files) {
      const path = join(directory, file.path);
      await mkdir(dirname(path), { recursive: true, mode: 0o700 });
      await writeFile(path, Buffer.from(file.data, 'base64'), { flag: 'wx', mode: 0o600 });
    }
    etag = response.headers.get('etag');
    previous = digest(bytes);
  }
  return {
    async save({ roots } = {}) {
      const files = [];
      let size = 0;
      async function walk(relative = '') {
        for (const name of (await readdir(join(directory, relative))).sort()) {
          if (!relative && roots && !roots.has(name)) continue;
          const path = relative ? `${relative}/${name}` : name;
          if (!validPath(path)) throw new Error('Invalid local checkpoint path.');
          const stat = await lstat(join(directory, path));
          if (stat.isDirectory()) { await walk(path); continue; }
          if (!stat.isFile() || stat.nlink !== 1) throw new Error('Checkpoint refuses links or special files.');
          size += stat.size;
          if (size > checkpointExpandedLimit || files.length >= 4096) throw Object.assign(new Error('Checkpoint exceeds limit.'), {checkpointReason: files.length >= 4096 ? 'file-limit' : 'expanded-limit'});
          files.push({ path, data: (await readFile(join(directory, path))).toString('base64') });
        }
      }
      await walk();
      const bytes = JSON.stringify({ version: 1, files });
      if (Buffer.byteLength(bytes) > checkpointExpandedLimit) throw Object.assign(new Error('Checkpoint exceeds limit.'), {checkpointReason:'expanded-limit'});
      const next = digest(bytes);
      if (next === previous) return;
      const compressed = gzipSync(bytes);
      if (compressed.length > checkpointLimit) throw Object.assign(new Error('Checkpoint exceeds limit.'), {checkpointReason:'compressed-limit'});
      const saved = await request(endpoint, { method: 'PUT', body: compressed, redirect: 'error',
        signal: AbortSignal.timeout(30000), headers: { 'content-type': 'application/gzip',
          ...(etag ? { 'if-match': etag } : { 'if-none-match': '*' }) } });
      if (!saved.ok || !saved.headers.get('etag')) throw Object.assign(new Error('Checkpoint write failed; stop this runtime before retrying.'), {checkpointReason:'storage-rejected',checkpointStatus:saved.status});
      etag = saved.headers.get('etag');
      previous = next;
    },
  };
}

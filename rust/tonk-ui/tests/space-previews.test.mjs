import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { previewRequest } from '../../tonk-portal/src/preview_cache.js';

const host = { isConnected: true };
const profile = { with: 'account-a@profile:tonk', repo: '', branch: 'account-a' };
const space = { with: 'main@did:key:a', repo: 'did:key:a', branch: 'main', preview: 'did:key:a' };
const image = 'data:image/webp;base64,UklGRg==';

const library = readFileSync(new URL('../../tonk-core/assets/library/profile.yaml', import.meta.url), 'utf8');
const source = library.split('element!: &hub-collection\n')[1].split('    previews: |\n')[1].split('\n    connected: |')[0];
const loadPreviews = Function(`return (${source.trim()})`)();

test('hub keeps fallback on misses, reveals decoded images, and reloads a reused card identity', async () => {
  const attributes = new Set(['data-preview-pending']);
  const imageNode = { dataset: { spaceThumbnail: 'did:key:a' }, isConnected: true,
    setAttribute(name) { attributes.add(name); },
    hasAttribute(name) { return attributes.has(name); },
    removeAttribute(name) { attributes.delete(name); if (name === 'src') delete this.src; } };
  const collection = { isConnected: true, querySelectorAll: () => [imageNode] };
  let calls = 0;
  globalThis.window = { tonk: { preview: async ({ repos }) => {
    calls++; return repos.includes('did:key:b') ? { 'did:key:b': image } : {};
  } } };
  await loadPreviews(collection);
  assert.equal(imageNode.hasAttribute('data-preview-pending'), true);
  await loadPreviews(collection);
  assert.equal(calls, 1, 'a cache miss does not repeatedly query on DOM mutations');
  imageNode.dataset.spaceThumbnail = 'did:key:b';
  await loadPreviews(collection);
  assert.equal(imageNode.src, image);
  assert.equal(imageNode.hasAttribute('data-preview-pending'), true, 'keep fallback until decoded');
  imageNode.onload();
  assert.equal(imageNode.hasAttribute('data-preview-pending'), false);
  imageNode.onerror();
  assert.equal(imageNode.hasAttribute('data-preview-pending'), true);
});

test('preview cache binds writes to space and reads to the profile across nested relays', async () => {
  const forwarded = [];
  globalThis.window = { parent: {}, tonk: { preview: async r => { forwarded.push(r); return null; } } };
  const put = { action: 'put', repo: 'did:key:a', image };
  await previewRequest(put, space, host);
  assert.equal(forwarded.length, 1);
  await previewRequest({ ...put, repo: 'did:key:b' }, space, host);
  await previewRequest({ action: 'get', repos: ['did:key:b'] }, space, host);
  await previewRequest(put, { ...space, preview: '' }, host);
  await previewRequest(put, { ...space, branch: 'other' }, host);
  await previewRequest(put, space, { isConnected: false });
  await previewRequest({ ...put, image: 'data:image/svg+xml,<svg/>' }, space, host);
  await previewRequest({ ...put, image: image + 'A'.repeat(48000) }, space, host);
  assert.equal(forwarded.length, 1, 'refused requests never cross the next boundary');
  await previewRequest(put, { with: '', repo: '', preview: 'did:key:a' }, host);
  assert.equal(forwarded.length, 2, 'implicit content portal inherits only its home identity');
});

test('tab cache survives reload, throttles, expires, caps entries and clears on account change', async () => {
  globalThis.window = {}; window.parent = window;
  const storage = new Map();
  globalThis.sessionStorage = {
    getItem: k => storage.get(k) ?? null,
    setItem: (k, v) => storage.set(k, v), removeItem: k => storage.delete(k),
  };
  const originalNow = Date.now;
  let now = 100000; Date.now = () => now;
  const put = (repo, value = image) => previewRequest({ action: 'put', repo, image: value }, profile, host);
  const get = (repos, context = profile) => previewRequest({ action: 'get', repos }, context, host);
  try {
    await put('did:key:a');
    await put('did:key:a', image + 'AA');
    assert.equal((await get(['did:key:a']))['did:key:a'], image, 'one write per minute');
    const reloaded = await import('../../tonk-portal/src/preview_cache.js?reload');
    assert.equal((await reloaded.previewRequest({ action: 'get', repos: ['did:key:a'] }, profile, host))['did:key:a'], image);
    for (let i = 0; i < 40; i++) { now += 1000; await put(`did:key:${i}`); }
    assert.equal(Object.keys(JSON.parse([...storage.values()][0]).entries).length, 32);
    now += 86400001;
    assert.deepEqual(await get(['did:key:39']), {});
    await put('did:key:a');
    assert.deepEqual(await get(['did:key:a'], { ...profile, with: 'account-b@profile:tonk' }), {});
    assert.deepEqual(await get(['did:key:a']), {}, 'old account cache was discarded');
    sessionStorage.setItem = () => { throw new Error('quota'); };
    await put('did:key:quota');
    assert.equal((await get(['did:key:quota']))['did:key:quota'], image, 'storage failure is best effort');
  } finally { Date.now = originalNow; }
});

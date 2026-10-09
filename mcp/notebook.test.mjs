import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readNotebook, notebookURL } from './notebook.mjs';

function fixture({ missing = false, duplicate = false } = {}) {
  const entity = 'urn:notebook:one';
  const calls = [];
  const backend = { webOrigin: 'https://tonk.foundation', async call(name, args) {
    calls.push({ name, args });
    if (name === 'tonk_space_info') return { subject: 'did:key:z6Test' };
    return { revision: { edition: 12 }, matches: [
      { results: [{ this: entity, fields: { title: 'A real notebook' } }] },
      { results: [{ this: entity, fields: { block: { N9: 'urn:block:two', N1: 'urn:block:one' } } }] },
      { results: [
        { this: 'urn:block:two', fields: { notebook: entity, source: 'Second' } },
        ...(!missing ? [{ this: 'urn:block:one', fields: { notebook: entity, source: '# First' } }] : []),
        { this: 'urn:block:orphan', fields: { notebook: entity, source: 'Do not show as placed' } },
        ...(duplicate ? [{ this: 'urn:block:two', fields: { notebook: entity, source: 'Conflicting text' } }] : []),
      ] },
    ] };
  } };
  return { backend, entity, calls };
}

test('notebook presentation preserves sequence order, reports unplaced blocks and returns a host-bound link', async () => {
  const { backend, entity, calls } = fixture();
  const result = await readNotebook(backend, entity);
  assert.equal(result.markdown, '# First\n\nSecond');
  assert.deepEqual(result.blocks.map(block => block.position), ['N1', 'N9']);
  assert.equal(result.unplacedBlocks, 1);
  assert.equal(result.url, 'https://tonk.foundation/space/did%3Akey%3Az6Test/notebook/urn%3Anotebook%3Aone');
  assert.deepEqual(calls.map(call => call.name), ['tonk_query', 'tonk_space_info']);
  assert.equal(result.readOnly, true);
});

test('notebook presentation fails instead of inventing missing or conflicting content', async () => {
  for (const options of [{ missing: true }, { duplicate: true }]) {
    const { backend, entity } = fixture(options);
    await assert.rejects(readNotebook(backend, entity), /missing|Ambiguous/);
  }
});

test('entity injection and caller-supplied URL destinations never reach the runtime', async () => {
  const { backend, calls } = fixture();
  for (const entity of ['urn:note\nprose!:', 'https://evil.test/?token=x#secret', 'urn:note"', 'urn:note\\', 'urn:note\u0000']) {
    await assert.rejects(readNotebook(backend, entity), /exact notebook entity/);
  }
  assert.equal(calls.length, 0);
  assert.equal(notebookURL(undefined, 'did:key:z6Test', 'urn:note:one'), undefined);
  assert.equal(notebookURL('https://tonk.foundation', 'other-space', 'urn:note:one'), undefined);
  assert.throws(() => notebookURL('https://tonk.foundation/extra', 'did:key:z6Test', 'urn:note:one'));
});

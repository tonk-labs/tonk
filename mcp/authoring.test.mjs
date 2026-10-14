import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { authoringGuide } from './authoring.mjs';
test('bundled authoring manuals match canonical notation, views and events', async () => {
  for (const [topic, path] of [
    ['notation', '../rust/tonk-notation/guide.md'],
    ['views', '../rust/tonk-cli/src/guide-views.md'],
    ['events', '../rust/tonk-cli/src/guide-events.md'],
  ])
    assert.equal(
      authoringGuide({ topic }).text,
      await readFile(new URL(path, import.meta.url), 'utf8'),
    );
  assert.ok(authoringGuide().topics.includes('notation'));
  assert.throws(() => authoringGuide({ topic: '../../private' }));
});

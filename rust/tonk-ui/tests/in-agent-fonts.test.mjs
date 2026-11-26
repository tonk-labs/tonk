// Every font the in-agent stylesheet names ships with the app, under
// /in-agent/fonts, with its licence: the cards Tonk draws in agent apps
// load them from there (rust/tonk-worker/src/router/hosted/in_agent.rs).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, access } from 'node:fs/promises';

const ROOT = new URL('../../', import.meta.url);
const FONTS = new URL('tonk-ui/assets/in-agent/fonts/', ROOT);

test('every font the in-agent stylesheet names is shipped with its licence', async () => {
  const css = await readFile(
    new URL('tonk-worker/src/router/hosted/in-agent/in-agent.css', ROOT),
    'utf8',
  );
  const names = [...css.matchAll(/url\("__TONK_IN_AGENT_FONTS__\/([\w.-]+)"\)/g)].map((m) => m[1]);
  assert.equal(names.length, 5, 'the five Plex faces');
  for (const name of [...names, 'OFL.txt']) await access(new URL(name, FONTS));
});

test('the fonts are served with CORS, as a card loads them from its own origin', async () => {
  const headers = await readFile(new URL('tonk-ui/assets/_headers', ROOT), 'utf8');
  const rule = headers.split(/\n(?=\/)/).find((block) => block.startsWith('/in-agent/*'));
  assert.ok(rule, 'no /in-agent/* rule');
  assert.match(rule, /Access-Control-Allow-Origin: \*/);
});

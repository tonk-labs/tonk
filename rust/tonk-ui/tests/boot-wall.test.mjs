// The failed start is a wall (gooey task-page.html#no-start): the reason is
// an error message, so it carries the error mark, and focus goes to it to be
// read out without a ring drawn round it.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const index = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const rule = (selector) => {
  const start = index.indexOf(`${selector} {`);
  assert.ok(start >= 0, `index.html styles ${selector}`);
  return index.slice(start, index.indexOf('}', start));
};

test('the failed start reason draws no focus ring', () => {
  assert.match(rule('#tonk-boot .tonk-boot-status[data-failed]'), /outline: none;/);
});

test('the failed start reason carries the error mark', () => {
  const mark = rule('#tonk-boot .tonk-boot-status[data-failed]::before');
  assert.match(mark, /mask: var\(--tonk-boot-alert\)/);
  assert.match(index, /--tonk-boot-alert: url\("data:image\/svg\+xml,.*circle cx='12' cy='12' r='10'/);
  assert.match(rule('#tonk-boot .tonk-boot-status[data-failed]'), /padding: 16px 16px 16px 44px;/);
});

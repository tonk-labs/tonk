import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

test('vendored template hashes, installable seeds and Hub cards agree', () => {
  execFileSync(process.execPath, [
    fileURLToPath(new URL('../../../scripts/build-discover.mjs', import.meta.url)),
    '--check',
  ], { stdio: 'pipe' });
});

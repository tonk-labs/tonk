import { test } from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';
import { renderWorkerUI } from '../apps/ui.mjs';

test('hosted executable bundles against the core without retired runtimes or Desktop', async () => {
  const result = await build({
    entryPoints: [fileURLToPath(new URL('./server.mjs', import.meta.url))],
    bundle: true,
    platform: 'node',
    format: 'esm',
    packages: 'external',
    write: false,
    metafile: true,
  });
  const modules = Object.keys(result.metafile.inputs);
  assert.ok(modules.some((path) => path.endsWith('agent-link-backend.mjs')));
  assert.ok(modules.some((path) => path.endsWith('hosted/agent-service.mjs')));
  assert.ok(modules.some((path) => path.endsWith('hosted/sponsor-service.mjs')));
  assert.ok(modules.some((path) => path.endsWith('hosted/oauth.mjs')));
  // OAuth sponsors an agent; it never attaches the human's own account.
  assert.ok(!modules.some((path) => /hosted\/worker-tenants\.mjs|account-backend\.mjs/.test(path)));
  assert.ok(
    modules
      .filter((path) => path.includes('/chatgpt/'))
      .every((path) => path.endsWith('chatgpt/metadata.mjs')),
  );

  assert.ok(!modules.some((path) => /legacy|desktop|native\.mjs|runtime-client/.test(path)));
  const html = renderWorkerUI({ origin: 'https://mcp.example.test' });
  const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
  assert.equal(scripts.length, 1);
  assert.doesNotThrow(() => new Function(scripts[0][1]));
});

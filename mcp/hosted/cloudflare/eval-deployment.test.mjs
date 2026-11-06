import { test } from 'node:test';
import assert from 'node:assert/strict';
import { evalDeployment } from './eval-deployment.mjs';

const base = {
  suffix: 'eval-storage-admission-20261020',
  provider: 'https://tonk.foundation',
  subdomain: 'tonk',
  accountId: 'a'.repeat(32),
  commit: 'b'.repeat(40),
};

test('evaluation deployment gets a fresh workers.dev issuer and private storage', () => {
  const config = evalDeployment(base);
  assert.equal(config.name, 'tonk-mcp-eval-storage-admission-20261020');
  assert.equal(
    config.vars.TONK_MCP_ISSUER,
    'https://tonk-mcp-eval-storage-admission-20261020.tonk.workers.dev',
  );
  assert.equal(config.vars.TONK_PROVIDER_ORIGIN, 'https://tonk.foundation');
  assert.equal(
    config.r2_buckets[0].bucket_name,
    'tonk-mcp-eval-storage-admission-20261020-checkpoints',
  );
  assert.equal(config.containers[0].max_instances, 1);
  assert.equal(config.workers_dev, true);
  assert.deepEqual(config.routes, []);
  assert.equal(config.observability.enabled, false);
});

test('evaluation deployment rejects unsafe suffixes, shared providers, and mutable identifiers', () => {
  for (const override of [
    { suffix: 'eval-x' },
    { suffix: 'agent-dev' },
    { suffix: 'eval-storage/' },
    { provider: 'https://agent-dev.tonk.host' },
    { provider: 'https://tonk.foundation/path' },
    { subdomain: 'tonk/path' },
    { commit: 'main' },
  ])
    assert.throws(() => evalDeployment({ ...base, ...override }));
});

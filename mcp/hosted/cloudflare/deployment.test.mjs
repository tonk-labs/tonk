import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deployment } from './deployment.mjs';
const base = { accountId: 'a'.repeat(32), commit: 'b'.repeat(40), subdomain: 'tonk' };
test('deployment channels bind distinct issuers, storage and matching providers', () => {
  const configs = [
    {
      channel: 'preview',
      pullRequest: 1091,
      provider: 'https://pr-1091-tonk-access-service-preview.tonk.workers.dev',
    },
    {
      channel: 'preview',
      pullRequest: 1092,
      provider: 'https://pr-1092-tonk-access-service-preview.tonk.workers.dev',
    },
    { channel: 'dev', provider: 'https://tonk.foundation' },
    { channel: 'staging', provider: 'https://staging.tonk.xyz' },
    { channel: 'stable', provider: 'https://tonk.network' },
  ].map((input) => deployment({ ...base, ...input }));
  for (const field of ['TONK_MCP_ISSUER'])
    assert.equal(new Set(configs.map((config) => config.vars[field])).size, configs.length);
  assert.equal(
    new Set(configs.map((config) => config.r2_buckets[0].bucket_name)).size,
    configs.length,
  );
  for (const config of configs) {
    assert.equal(config.containers[0].class_name, 'TonkRuntime');
    assert.deepEqual(config.migrations, [{ tag: 'v1', new_sqlite_classes: ['TonkRuntime'] }]);
    assert.equal(config.vars.TONK_MCP_CLIENTS, undefined);
    assert.equal(config.vars.TONK_MCP_CLIENT_ID, undefined);
    assert.equal(config.vars.TONK_MCP_SPACE, undefined);
    assert.equal(config.vars.TONK_MCP_DCR, undefined);
  }
  const [pr, , dev, staging, stable] = configs;
  assert.equal(pr.vars.TONK_MCP_ISSUER, 'https://tonk-mcp-pr-1091.tonk.workers.dev');
  assert.equal(pr.workers_dev, true);
  assert.deepEqual(pr.routes, []);
  for (const [config, domain] of [
    [dev, 'agent-dev.tonk.host'],
    [staging, 'agent-staging.tonk.host'],
    [stable, 'agent.tonk.host'],
  ]) {
    assert.equal(config.vars.TONK_MCP_ISSUER, `https://${domain}`);
    assert.equal(config.workers_dev, false);
    assert.deepEqual(config.routes, [{ pattern: domain, custom_domain: true }]);
    // One-level names only: the tonk.host zone has no nested subdomains.
    assert.equal(domain.split('.').length <= 3, true);
  }
});
test('deployment rejects cross-channel providers and malformed input before provisioning', () => {
  const valid = {
    ...base,
    channel: 'preview',
    pullRequest: 1091,
    provider: 'https://pr-1091-tonk-access-service-preview.tonk.workers.dev',
  };
  for (const override of [
    { provider: 'https://tonk.network' },
    { pullRequest: '../1091' },
    { subdomain: 'tonk/path' },
    { commit: 'main' },
    { channel: 'production' },
    { channel: 'stable' },
    { provider: valid.provider + '/settings' },
  ])
    assert.throws(() => deployment({ ...valid, ...override }));
});

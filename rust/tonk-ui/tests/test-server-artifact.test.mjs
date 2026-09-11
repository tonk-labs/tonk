import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdtempSync, mkdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

// Execute the actual Nix shell body with only store references substituted.
function fixture(run) {
  const root = mkdtempSync(join(tmpdir(), 'tonk-artifact-contract-'));
  try {
    const artifact = join(root, 'selected artifact');
    const deployment = join(root, 'deployment');
    mkdirSync(artifact);
    mkdirSync(deployment);
    writeFileSync(join(artifact, 'index.html'), 'selected-index');
    writeFileSync(join(artifact, 'service_worker.js'), 'selected-worker');
    const caddy = join(root, 'caddy');
    writeFileSync(caddy, '#!/bin/sh\ncat\n', { mode: 0o755 });
    const flake = readFileSync(new URL('../../../flake.nix', import.meta.url), 'utf8');
    const body = flake.split('writeScriptBin "tonk-ui-test-server" \'\'')[1].split("\n            '';")[0]
      .replaceAll('${bash}/bin/bash', '/bin/bash')
      .replaceAll('${self.packages.${system}.tonk-ui}', '/unavailable-baked-artifact')
      .replaceAll('${caddy}/bin/caddy', caddy)
      .replaceAll("''${", '${');
    const script = join(root, 'server.sh');
    writeFileSync(script, body);
    run({ artifact, deployment, script });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test('selected artifact is installed before the server announces readiness', () => fixture(({ artifact, deployment, script }) => {
  const result = spawnSync('/bin/bash', [script, '8080', '8090', deployment], {
    env: { ...process.env, TONK_UI_TEST_ARTIFACT: artifact }, encoding: 'utf8',
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(join(deployment, 'current', 'index.html'), 'utf8'), 'selected-index');
  assert.equal(readFileSync(join(deployment, 'current', 'service_worker.js'), 'utf8'), 'selected-worker');
  assert.ok(result.stdout.includes(`root * "${deployment}/current"`));
}));

test('invalid selection fails without announcing readiness or using the baked artifact', () => fixture(({ deployment, script }) => {
  const result = spawnSync('/bin/bash', [script, '8080', '8090', deployment], {
    env: { ...process.env, TONK_UI_TEST_ARTIFACT: '/nonexistent-selected-artifact' }, encoding: 'utf8',
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Invalid Tonk test artifact/);
  assert.ok(!result.stdout.includes('Test server live'));
}));

test('copy failure cannot announce a ready deployment', () => fixture(({ artifact, deployment, script }) => {
  // A file at generation-a makes the real mkdir fail before Caddy can start.
  writeFileSync(join(deployment, 'generation-a'), 'occupied');
  const result = spawnSync('/bin/bash', [script, '8080', '8090', deployment], {
    env: { ...process.env, TONK_UI_TEST_ARTIFACT: artifact }, encoding: 'utf8',
  });
  assert.notEqual(result.status, 0);
  assert.ok(!result.stdout.includes('Test server live'));
}));

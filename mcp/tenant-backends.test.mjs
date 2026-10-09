import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, rm, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createTenantBackends } from './tenant-backends.mjs';

test('tenant resolver binds OAuth identity and host selection, sharing only the same pinned backend', async () => {
  const root = await mkdtemp(join(tmpdir(), 'tonk-tenant-resolver-'));
  let opened = 0, closed = 0, selections = 0;
  const principal = { tenantId: 'tenant-ABC123', deviceDid: 'did:key:device', rootDid: 'did:key:account' };
  let subject = 'did:key:space';
  await mkdir(join(root, principal.tenantId), { mode: 0o700 });
  const backend = { capabilities: ['tonk_query'], close: async () => { closed++; } };
  const resolver = createTenantBackends({ binary: 'unused', dataRoot: root, capacity: 1,
    chooseSubject: async () => subject,
    startAccount: async options => {
      opened++;
      assert.equal(options.enableSpaces, true);
      return { status: async () => principal, close: async () => { closed++; },
        openSpace: async selected => { selections++; assert.equal(selected, subject); return backend; } };
    },
  });
  try {
    const [a, b] = await Promise.all([resolver.resolve(principal), resolver.resolve(principal)]);
    assert.equal(a, b); assert.equal(a, backend);
    assert.equal(opened, 1); assert.equal(selections, 1);
    await assert.rejects(resolver.resolve({ ...principal, rootDid: 'did:key:other' }), /pinned/);
    subject = 'did:key:other-space';
    await assert.rejects(resolver.resolve(principal), /pinned/);
    await assert.rejects(resolver.resolve({ ...principal, tenantId: '../escape' }), /Invalid tenant/);
    await assert.rejects(resolver.resolve({ ...principal, tenantId: 'tenant-XYZ456' }), /capacity/);
  } finally { await resolver.close(); await rm(root, { recursive: true, force: true }); }
  assert.equal(closed, 1);
  await assert.rejects(resolver.resolve(principal), /closed/);
});

test('tenant resolver closes mismatched native identity and refuses a symlink before starting it', async () => {
  const root = await mkdtemp(join(tmpdir(), 'tonk-tenant-mismatch-'));
  const principal = { tenantId: 'tenant-ABC123', deviceDid: 'did:key:device', rootDid: 'did:key:account' };
  let opened = 0, closed = 0;
  await mkdir(join(root, principal.tenantId));
  const resolver = createTenantBackends({ binary: 'unused', dataRoot: root,
    chooseSubject: async () => 'did:key:space',
    startAccount: async () => {
      opened++;
      return { status: async () => ({ ...principal, deviceDid: 'did:key:other' }),
        close: async () => { closed++; }, openSpace: async () => { assert.fail('must not open space'); } };
    },
  });
  try {
    await assert.rejects(resolver.resolve(principal), /does not match/);
    assert.equal(closed, 1);
    await rm(join(root, principal.tenantId), { recursive: true });
    await symlink(tmpdir(), join(root, principal.tenantId));
    await assert.rejects(resolver.resolve(principal), /Invalid tenant directory/);
    assert.equal(opened, 1);
  } finally { await resolver.close(); await rm(root, { recursive: true, force: true }); }
});

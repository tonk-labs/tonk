import { test } from 'node:test';
import assert from 'node:assert/strict';
import { checkpointedProvisioner } from './provisioning.mjs';

test('checkpointed authorization retains active tenants and recovers the queue after failure', async () => {
  const events = [],
    diagnostics = [];
  let fail = true,
    active = false;
  const provision = checkpointedProvisioner({
    provision: async () => ({
      tenantId: 'new',
      authorize: async () => {
        assert.equal(active, false);
        active = true;
        await Promise.resolve();
        events.push('authorized');
        active = false;
        return { rootDid: 'root' };
      },
    }),
    retainedTenants: () => ['retained', 'retained'],
    checkpoint: {
      save: async ({ roots }) => {
        events.push('checkpoint');
        assert.deepEqual([...roots], ['new', 'retained']);
        if (fail) {
          fail = false;
          throw Error('private checkpoint details');
        }
      },
    },
    onFailure: (detail) => diagnostics.push(detail),
  });
  const account = await provision();
  const outcomes = await Promise.allSettled([account.authorize({}), account.authorize({})]);
  assert.equal(outcomes[0].reason.authorizationStage, 'checkpoint');
  assert.equal(outcomes[1].status, 'fulfilled');
  assert.deepEqual(events, ['authorized', 'checkpoint', 'authorized', 'checkpoint']);
  assert.ok(!JSON.stringify(diagnostics).includes('private'));
});

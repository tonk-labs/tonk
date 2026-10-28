import test from 'node:test';
import assert from 'node:assert/strict';
import { delegationParties } from './delegation-parties.mjs';

// Minted by tonk-identity `mint_addressed_device_delegation` with fixed seeds
// (root [1; 32], device [2; 32], remote https://hub.test/ucan/).
const fixture = {
  root: 'did:key:z6Mkon3Necd6NkkyfoGoHxid2znGc59LU3K7mubaRcFbLfLX',
  device: 'did:key:z6Mko9hTggMwjSTEaJaPUfE6tqcy2xvU6BnNq3e3o8qVBiyH',
  hex: 'a16663746e2d7631815901368258405e060f9df267938da7fbf7ed550d3310fa84fe2da72bb680fe5915047e1d28e6f4f3b1a979f3c4ff8cf1cf33176841450821181eb4430bcb31b860836089700fa26168483401ed01ed011371737563616e2f646c6740312e302e302d72632e31a86361756478386469643a6b65793a7a364d6b6f39685467674d776a535445614a615055664536747163793278765536426e4e713365336f3871564269794863636d64612f63657870f66369737378386469643a6b65793a7a364d6b6f6e334e656364364e6b6b79666f476f48786964327a6e476335394c55334b376d756261526346624c664c5863706f6c8063737562f6646d657461a16c686f6d652e616464726573737668747470733a2f2f6875622e746573742f7563616e2f656e6f6e63655044db865ee2e4f2fa48841888a0668a44',
};

test('it reads the issuer and audience of a real device delegation', () => {
  assert.deepEqual(delegationParties(fixture.hex), {
    issuer: fixture.root,
    audience: fixture.device,
  });
});

test('it refuses malformed input', () => {
  for (const input of [undefined, '', 'zz', 'abc', fixture.hex.slice(0, -2), fixture.hex + '00'])
    assert.throws(() => delegationParties(input));
});

test('it refuses a container without exactly one delegation', () => {
  // {"ctn-v1": []}
  assert.throws(() => delegationParties('a16663746e2d763180'), /exactly one/);
});

// Stub agent identity for the first hosted OAuth slice. It proves the browser
// flow (client -> OAuth -> passkey approval in the Tonk app -> token) without
// holding any authority:
//
// - The agent DID is a fresh Ed25519 key whose private half is discarded at
//   once, so the account delegation the approval page mints to it is unusable.
// - The callback reads only the delegation's parties, unverified. The sponsor
//   DID is shown to the agent, and never used as authority.
//
// The real slice mints a persistent agent account in the native worker and a
// verified sponsor grant from a dedicated approval command.
import { generateKeyPairSync, randomBytes } from 'node:crypto';
import { delegationParties } from './delegation-parties.mjs';
import { authoringGuide } from '../authoring.mjs';
import { TonkToolError } from '../errors.mjs';

const ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function base58(bytes) {
  let value = BigInt('0x' + Buffer.from(bytes).toString('hex'));
  let out = '';
  while (value > 0n) {
    out = ALPHABET[Number(value % 58n)] + out;
    value /= 58n;
  }
  for (const byte of bytes) {
    if (byte !== 0) break;
    out = '1' + out;
  }
  return out;
}

// A did:key for a fresh Ed25519 key. The private key is never exported.
export function throwawayDid() {
  const { publicKey } = generateKeyPairSync('ed25519');
  const raw = Buffer.from(publicKey.export({ format: 'jwk' }).x, 'base64url');
  return 'did:key:z' + base58(Buffer.concat([Buffer.from([0xed, 0x01]), raw]));
}

// `provisionAccount` for `createTonkOAuth`.
export function sponsorStubProvisioner({ mint = throwawayDid } = {}) {
  return async () => {
    const agentDid = mint();
    return {
      tenantId: 'agent-' + randomBytes(9).toString('base64url'),
      deviceDid: agentDid,
      async authorize(payload) {
        let parties;
        try {
          parties = delegationParties(payload?.delegationHex);
        } catch (cause) {
          throw Object.assign(Error('Unreadable approval', { cause }), {
            authorizationStage: 'provider',
          });
        }
        if (parties.audience !== agentDid || parties.issuer === agentDid)
          throw Object.assign(Error('Approval names another agent'), {
            authorizationStage: 'device-check',
          });
        return { deviceDid: agentDid, rootDid: parties.issuer };
      },
    };
  };
}

// Tools for a stub principal: the guide, and an empty space list that names
// the agent and its sponsor so a live test can see who connected.
export function sponsorStubBackend(principal) {
  const capabilities = ['tonk_guide', 'tonk_list_spaces'];
  return {
    spaceSelection: true,
    capabilities,
    async call(name, args) {
      if (name === 'tonk_guide') return authoringGuide(args);
      if (name === 'tonk_list_spaces')
        return {
          spaces: [],
          agent: principal.deviceDid,
          sponsor: principal.rootDid,
          note: 'Test connection: this agent has no spaces and cannot create them yet.',
        };
      throw new TonkToolError('Tool unavailable.');
    },
  };
}

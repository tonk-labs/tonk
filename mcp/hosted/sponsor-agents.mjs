// Sponsored hosted agents for the OAuth endpoint. Each connection is an agent
// with its own persistent key (kept by the native worker); the person who
// approves sponsors it from their own Tonk device, which grants the agent
// `/provider/add` on their account and nothing else. The native worker
// verifies that grant against the agent's key before it is kept.
import { authoringGuide } from '../authoring.mjs';
import { TonkToolError } from '../errors.mjs';

const stage = (name, message) => Object.assign(Error(message), { authorizationStage: name });
const DID = /^did:key:z[1-9A-HJ-NP-Za-km-z]{40,60}$/;

// `provisionAccount` for `createTonkOAuth`. `agents` is the native agent store
// (`createAgentService().agents`). `hint` names this browser's last agent for
// the same client, so a reconnect keeps the same agent.
export function agentProvisioner(agents) {
  return async ({ hint } = {}) => {
    let agent;
    if (hint) {
      try {
        agent = await agents.identity(hint);
      } catch {
        // An unknown or unreadable hint starts a new agent.
      }
    }
    agent ??= await agents.create();
    return {
      tenantId: agent.agent,
      deviceDid: agent.did,
      async authorize(payload) {
        if (
          !payload ||
          !DID.test(payload.account ?? '') ||
          typeof payload.recipient !== 'string' ||
          typeof payload.sponsorshipHex !== 'string'
        )
          throw stage('provider', 'Unreadable sponsorship');
        // A remembered agent belongs to whoever sponsored it first.
        if (agent.sponsor && agent.sponsor !== payload.account)
          throw stage('device-check', 'Another account sponsors this agent');
        let sponsored;
        try {
          sponsored = await agents.sponsor({
            agent: agent.agent,
            sponsorshipHex: payload.sponsorshipHex,
            recipient: payload.recipient,
          });
        } catch {
          throw stage('device-check', 'The sponsorship did not verify');
        }
        if (sponsored.sponsor !== payload.account || sponsored.did !== agent.did)
          throw stage('device-check', 'The sponsorship names another account');
        return { deviceDid: agent.did, rootDid: sponsored.sponsor, hint: agent.agent };
      },
    };
  };
}

// Tools for a sponsored agent. It has no spaces until slice 2 lets it create
// them; the list names the agent and its sponsor.
export function sponsoredAgentBackend(principal) {
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
          note: 'This agent has its own identity, sponsored by this account. It cannot create spaces yet.',
        };
      throw new TonkToolError('Tool unavailable.');
    },
  };
}

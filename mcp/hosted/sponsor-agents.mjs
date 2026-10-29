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

// Tools for a sponsored agent: the guide, its spaces, creating a space its
// sponsor owns, and outlining a space's views. Every call runs on the agent's own peer, which pulls
// before it reads and pushes after it writes.
export function sponsoredAgentBackend(principal, { agents, agentName }) {
  const capabilities = [
    'tonk_guide',
    'tonk_list_spaces',
    'tonk_create_space',
    'tonk_query',
    'tonk_evaluate',
    'tonk_outline',
  ];
  const agent = principal.tenantId;
  return {
    spaceSelection: true,
    capabilities,
    async call(name, args) {
      if (name === 'tonk_guide') return authoringGuide(args);
      if (name === 'tonk_list_spaces') {
        const listed = await agents.call({ agent, call: 'tonk_list_spaces' });
        return { ...listed, agent: principal.deviceDid, sponsor: principal.rootDid };
      }
      if (name === 'tonk_create_space') {
        const created = await agents.createSpace({ agent, name: args.name, agentName });
        return {
          space: created.space,
          name: created.name,
          claim_url: created.claim_url,
          next: 'Give the user claim_url and ask them to open it: it adds the space to their Tonk space list. Use space with tonk_query and tonk_evaluate.',
        };
      }
      if (name === 'tonk_query' || name === 'tonk_evaluate')
        return agents.call({ agent, call: name, space: args.space, document: args.document });
      if (name === 'tonk_outline')
        return agents.call({ agent, call: name, space: args.space, route: args.route });
      throw new TonkToolError('Tool unavailable.');
    },
  };
}

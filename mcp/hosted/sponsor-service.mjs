// OAuth endpoint where a person sponsors an agent connection. Each connection
// is a hosted agent with its own persistent key; approving in the Tonk app
// (`/settings/agent`) sponsors it. See `sponsor-agents.mjs`.
import { createDurableState } from './durable-state.mjs';
import { createTonkOAuth } from './oauth.mjs';
import { createAuthenticatedTonkHTTP } from './authenticated-http.mjs';
import { agentProvisioner, sponsoredAgentBackend } from './sponsor-agents.mjs';
import { providerOrigin } from './provider.mjs';
import { createSpaceSessions } from './space-session.mjs';
import { createWorkerUI } from '../apps/ui.mjs';
import { openAIResourceMetadata } from '../chatgpt/metadata.mjs';
import { TonkToolError } from '../errors.mjs';

const STATE_VERSION = 'sponsor-agents-1';

// Pre-registered public clients, for hosts that take a client ID. Others use
// dynamic registration.
export const knownClients = [
  {
    clientId: 'tonk-chatgpt',
    displayName: 'ChatGPT',
    redirectUris: ['https://chatgpt.com/connector_platform_oauth_redirect'],
    type: 'public',
    applicationType: 'web',
    enabled: true,
  },
  {
    clientId: 'tonk-claude',
    displayName: 'Claude',
    redirectUris: ['https://claude.ai/api/mcp/auth_callback'],
    type: 'public',
    applicationType: 'web',
    enabled: true,
  },
];

export async function createSponsorService({
  issuer,
  provider,
  now = Date.now,
  durable,
  agents,
  provision = agentProvisioner(agents),
  // How long one approval lasts; the client then asks the person again.
  connectionLifetime = 30 * 24 * 60 * 60_000,
}) {
  provider = providerOrigin(provider);
  durable ??= await createDurableState();
  // State from another version names agents this one does not keep (the
  // first slice's throwaway keys). Start over; those clients connect again.
  const saved = durable.state?.version === STATE_VERSION ? durable.state : undefined;
  const oauth = createTonkOAuth({
    issuer,
    now,
    clients: knownClients,
    dynamicRegistration: true,
    // A client registers once per connection and keeps its ID; an expired
    // registration would break every later refresh.
    registrationLifetime: 90 * 24 * 60 * 60_000,
    linkPage: provider + '/settings/agent',
    approvalParameter: 'agent',
    connectionLifetime,
    saved: saved?.oauth,
    provisionAccount: provision,
    onRevoke: (principal) => sessions.revokePrincipal(principal),
  });
  // A view in the chat reaches its space through the agent's open worker,
  // with a short-lived session that names one space. It writes as the agent.
  const sessions = createSpaceSessions({
    now,
    isPrincipalActive: (principal) => oauth.isPrincipalActive(principal),
    resolveWorker: (principal) => agents.view(principal.tenantId),
  });
  const http = createAuthenticatedTonkHTTP({
    oauth,
    now,
    registerUI: createWorkerUI({
      origin: issuer,
      resourceMetadata: openAIResourceMetadata(issuer),
    }),
    resolveBackend: async (principal) => ({
      ...sponsoredAgentBackend(principal, {
        agents,
        agentName: oauth.clientName(principal.clientId) ?? 'Agent',
      }),
      async openSpace(space) {
        const listed = await agents.call({ agent: principal.tenantId, call: 'tonk_list_spaces' });
        if (!listed?.spaces?.some((entry) => entry.subject === space))
          throw new TonkToolError(
            'That space is not one of yours. Call tonk_list_spaces and use an exact subject.',
          );
        await agents.view(principal.tenantId);
        return sessions.issue(principal, space);
      },
    }),
  });
  return {
    fetch(request) {
      return durable.run(
        () =>
          new URL(request.url).pathname.startsWith('/space-api/')
            ? sessions.fetch(request)
            : http.fetch(request),
        () => ({ version: STATE_VERSION, oauth: oauth.snapshot() }),
      );
    },
    async close() {
      sessions.close();
      await http.close();
    },
  };
}

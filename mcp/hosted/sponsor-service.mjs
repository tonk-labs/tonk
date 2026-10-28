// OAuth endpoint where a human sponsors an agent connection. This first slice
// uses the stub identity in `sponsor-stub.mjs`: it proves discovery,
// registration, passkey approval in the Tonk app and token issue, and grants
// the agent no authority.
import { createDurableState } from './durable-state.mjs';
import { createTonkOAuth } from './oauth.mjs';
import { createAuthenticatedTonkHTTP } from './authenticated-http.mjs';
import { sponsorStubBackend, sponsorStubProvisioner } from './sponsor-stub.mjs';
import { providerOrigin } from './provider.mjs';

const STATE_VERSION = 'sponsor-stub-1';

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
  provision = sponsorStubProvisioner(),
}) {
  provider = providerOrigin(provider);
  durable ??= await createDurableState();
  if (durable.state && durable.state.version !== STATE_VERSION)
    throw Error('Sponsor credential snapshot version mismatch');
  const oauth = createTonkOAuth({
    issuer,
    now,
    clients: knownClients,
    dynamicRegistration: true,
    // A client registers once per connection and keeps its ID; an expired
    // registration would break every later refresh.
    registrationLifetime: 90 * 24 * 60 * 60_000,
    linkPage: provider + '/settings/link',
    saved: durable.state?.oauth,
    provisionAccount: provision,
  });
  const http = createAuthenticatedTonkHTTP({
    oauth,
    now,
    resolveBackend: async (principal) => sponsorStubBackend(principal),
  });
  return {
    fetch(request) {
      return durable.run(
        () => http.fetch(request),
        () => ({ version: STATE_VERSION, oauth: oauth.snapshot() }),
      );
    },
    async close() {
      await http.close();
    },
  };
}

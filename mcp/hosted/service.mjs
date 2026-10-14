import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createDurableState } from './durable-state.mjs';
import { createCheckpoint } from './checkpoint.mjs';
import { createTonkOAuth } from './oauth.mjs';
import { createAuthenticatedTonkHTTP } from './authenticated-http.mjs';
import { workerAccountProvisioner, createWorkerTenants } from './worker-tenants.mjs';
import { accountBackend } from '../account-backend.mjs';
import { openAIResourceMetadata } from '../chatgpt/metadata.mjs';
import { createWorkerUI } from '../apps/ui.mjs';
import { createSpaceSessions } from './space-session.mjs';
import { checkpointedProvisioner } from './provisioning.mjs';
import { providerOrigin } from './provider.mjs';

export async function createHostedService({
  issuer,
  subject,
  binary,
  provider,
  clientId,
  redirectUri,
  clients,
  dynamicRegistration = false,
  dataRoot,
  checkpoint,
  durable,
  provision,
  tenants,
}) {
  provider = providerOrigin(provider);
  dataRoot ??= await mkdtemp(join(tmpdir(), 'tonk-worker-seeds-'));
  checkpoint ??= await createCheckpoint({ directory: dataRoot });
  durable ??= await createDurableState();
  if (durable.state && (durable.state.version !== 1 || durable.state.subject !== subject))
    throw Error('Credential snapshot configuration mismatch');
  provision ??= workerAccountProvisioner({ binary, dataRoot, provider });
  const oauth = createTonkOAuth({
    issuer,
    dynamicRegistration: dynamicRegistration,
    clientId: clientId,
    redirectUris: [redirectUri],
    clients: clients,
    linkPage: provider + '/settings/link',
    saved: durable.state?.oauth,
    onRevoke: (principal) => sessions.revokePrincipal(principal),
    provisionAccount: checkpointedProvisioner({
      provision,
      checkpoint,
      retainedTenants: () => [...oauth.retainedTenants(), ...sessions.retainedTenants()],
    }),
  });
  tenants ??= createWorkerTenants({ binary, dataRoot });
  const sessions = createSpaceSessions({
    saved: durable.state?.sessions,
    isPrincipalActive: (principal) => oauth.isPrincipalActive(principal),
    resolveWorker: (principal, selected) => tenants.resolve(principal, selected),
  });
  const http = createAuthenticatedTonkHTTP({
    oauth,
    registerUI: createWorkerUI({
      origin: issuer,
      resourceMetadata: openAIResourceMetadata(issuer),
    }),
    resolveBackend: async (principal) => ({
      ...accountBackend({ principal, tenants }),
      async openSpace(space) {
        await tenants.resolve(principal, space);
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
        () => ({ version: 1, subject, oauth: oauth.snapshot(), sessions: sessions.snapshot() }),
      );
    },
    async close() {
      sessions.close();
      await http.close();
      await tenants.close();
    },
  };
}

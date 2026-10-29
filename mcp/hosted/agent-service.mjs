import { mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { agentLinkBackend } from '../agent-link-backend.mjs';
import { createTonkHTTP } from '../http.mjs';
import { createCheckpoint } from './checkpoint.mjs';
import { createAgentViews } from './agent-views.mjs';

// The native store records each space by the absolute path of its storage, so
// a restored checkpoint must land at the same path in every container. A fresh
// temporary name per start would leave every space pointing at the last one.
export const agentStore = join(tmpdir(), 'tonk-agent-store');

// One bounded, serialized worker owner. Checkpoint only after the child closes,
// including failure paths; a failed checkpoint freezes subsequent calls.
export async function createAgentService({
  issuer,
  provider,
  binary,
  directory,
  checkpoint,
  run,
  viewOptions = {},
}) {
  if (!directory) {
    directory = agentStore;
    await mkdir(directory, { recursive: true, mode: 0o700 });
  }
  checkpoint ??= await createCheckpoint({
    directory,
    endpoint: 'http://checkpoint.internal/agent-state',
  });
  const backend = agentLinkBackend({
    binary,
    dataDirectory: directory,
    remote: provider + '/ucan/',
    linkOrigin: provider,
    checkpoint,
    run,
  });
  const handler = createTonkHTTP(async () => backend);
  const views = createAgentViews({
    binary,
    directory,
    remote: provider + '/ucan/',
    exclusive: (task) => backend.exclusive(task),
    ...viewOptions,
  });
  return {
    // Persistent hosted agents, under the same root and checkpoint.
    agents: {
      create: () => backend.agent({ tool: 'agent_create' }),
      identity: (agent) => backend.agent({ tool: 'agent_identity', agent }),
      // Changes to the agent itself close its views first: the open copy
      // would not see them.
      sponsor: async ({ agent, sponsorshipHex, recipient }) => {
        await views.stop(agent);
        return backend.agent({
          tool: 'agent_sponsor',
          agent,
          sponsorship_hex: sponsorshipHex,
          recipient,
        });
      },
      createSpace: async ({ agent, name, agentName }) => {
        await views.stop(agent);
        return backend.agent({ tool: 'agent_create_space', agent, name, agent_name: agentName });
      },
      async call({ agent, call, space, document, route }) {
        const open = views.running(agent);
        if (open) return (await open).call(call, { space, document, route });
        return backend.agent({ tool: 'agent_call', agent, call, space, document, route });
      },
      // The agent's worker, held open for views of its spaces.
      view: (agent) => views.open(agent),
    },
    async fetch(request) {
      const url = new URL(request.url);
      if (url.origin !== issuer) return new Response(null, { status: 421 });
      if (url.pathname !== '/mcp/link' || url.search) return new Response(null, { status: 404 });
      if (request.headers.has('origin')) return new Response(null, { status: 403 });
      if (request.method !== 'POST') return new Response(null, { status: 405 });
      return handler.fetch(request);
    },
    async close() {
      await views.close();
      // Finish the active bounded invocation and its checkpoint. Do not start
      // queued work during rollout, or kill a write because its client closed.
      await backend.close({ drain: false });
      await handler.close();
    },
  };
}

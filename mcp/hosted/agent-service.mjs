import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { agentLinkBackend } from '../agent-link-backend.mjs';
import { createTonkHTTP } from '../http.mjs';
import { createCheckpoint } from './checkpoint.mjs';

// One bounded, serialized worker owner. Checkpoint only after the child closes,
// including failure paths; a failed checkpoint freezes subsequent calls.
export async function createAgentService({ issuer, provider, binary, directory, checkpoint, run }) {
  directory ??= await mkdtemp(join(tmpdir(), 'tonk-agent-workers-'));
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
  return {
    // Persistent hosted agents, under the same root and checkpoint.
    agents: {
      create: () => backend.agent({ tool: 'agent_create' }),
      identity: (agent) => backend.agent({ tool: 'agent_identity', agent }),
      sponsor: ({ agent, sponsorshipHex, recipient }) =>
        backend.agent({
          tool: 'agent_sponsor',
          agent,
          sponsorship_hex: sponsorshipHex,
          recipient,
        }),
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
      // Finish the active bounded invocation and its checkpoint. Do not start
      // queued work during rollout, or kill a write because its client closed.
      await backend.close({ drain: false });
      await handler.close();
    },
  };
}

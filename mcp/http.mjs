import { createMcpHandler } from '@modelcontextprotocol/server';
import { createTonkServer } from './core.mjs';

// Hosts supply an authorized backend or one that validates scoped credentials
// on every invocation. Never publish a singleton personal account worker.
export function createTonkHTTP(resolveBackend, { registerUI } = {}) {
  const handler = createMcpHandler(
    async (context) => {
      const backend = await resolveBackend(context.requestInfo);
      return createTonkServer(backend, { registerUI: (server) => registerUI?.(server, backend) });
    },
    { maxRequestBodySize: 200000, onerror: () => {} },
  );
  return handler;
}

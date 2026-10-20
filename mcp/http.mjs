import { createMcpHandler } from '@modelcontextprotocol/server';
import { createTonkServer } from './core.mjs';

// Resolve authority before constructing the backend. Production supplies an
// OAuth-authenticated, account-scoped resolver; no singleton personal account.
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

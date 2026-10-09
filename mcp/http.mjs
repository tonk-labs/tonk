import { createMcpHandler } from '@modelcontextprotocol/server';
import { createTonkServer } from './core.mjs';
import { registerWorkerUI } from './worker-ui.mjs';
import { registerQueryUI } from './ui.mjs';

// Resolve authority before constructing the backend. Production supplies an
// OAuth-authenticated, account-scoped resolver; no singleton personal account.
export function createTonkHTTP(resolveBackend) {
  const handler = createMcpHandler(async context => {
    const backend = await resolveBackend(context.requestInfo);
    return createTonkServer(backend, { registerUI: server => backend.openSpace ? registerWorkerUI(server, backend) : registerQueryUI(server, backend) });
  }, { maxRequestBodySize: 200000, onerror: () => {} });
  return handler;
}

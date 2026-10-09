import { createTonkHTTP } from './http.mjs';

// Each OAuth principal gets its own SDK handler. Even legacy MCP session IDs
// cannot select another tenant. Authenticate every request before resolution.
export function createAuthenticatedTonkHTTP({ oauth, resolveBackend }) {
  const handlers = new Map();
  let closed = false;
  return {
    async fetch(request) {
      if (closed) return new Response(null, { status: 503 });
      if (new URL(request.url).pathname !== '/mcp') return oauth.fetch(request);
      if (request.headers.has('origin')) return new Response(null, { status: 403 });
      const principal = oauth.authenticate(request);
      if (!principal) return oauth.challenge();
      if (request.method !== 'POST') return new Response(null, { status: 405, headers: { allow: 'POST' } });
      // Handler lifetime is one HTTP request: no unbounded tenant/session cache.
      // Track it until the finite HTTP response has been consumed.
      const handler = createTonkHTTP(() => resolveBackend(principal));
      const id = Symbol();
      handlers.set(id, handler);
      try {
        const response = await handler.fetch(request);
        // SDK per-request HTTP returns a finite result, including SSE POSTs.
        // GET streaming subscriptions are not enabled in this spike.
        const bytes = await response.arrayBuffer();
        return new Response(response.status === 204 || response.status === 304 ? null : bytes,
          { status: response.status, headers: response.headers });
      } finally { handlers.delete(id); await handler.close(); }
    },
    async close() {
      closed = true;
      oauth.close();
      await Promise.allSettled([...handlers.values()].map(handler => handler.close()));
      handlers.clear();
    },
  };
}

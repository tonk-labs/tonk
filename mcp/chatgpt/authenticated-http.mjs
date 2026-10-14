import { randomUUID } from 'node:crypto';
import {
  CLIENT_CAPABILITIES_META_KEY,
  isLegacyRequest,
  WebStandardStreamableHTTPServerTransport,
} from '@modelcontextprotocol/server';
import { createTonkHTTP } from '../http.mjs';
import { createTonkServer } from '../core.mjs';

const supportsApps = (capabilities) => {
  const types = capabilities?.extensions?.['io.modelcontextprotocol/ui']?.mimeTypes;
  return Array.isArray(types) && types.includes('text/html;profile=mcp-app');
};
const binding = (principal) =>
  JSON.stringify([
    principal.grantId,
    principal.clientId,
    principal.tenantId,
    principal.deviceDid,
    principal.rootDid,
    principal.connectionExpiresAt,
  ]);
async function bounded(stream, limit) {
  if (!stream) return new Uint8Array();
  const reader = stream.getReader(),
    chunks = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > limit) {
        await reader.cancel();
        throw Error('limit');
      }
      chunks.push(value);
    }
    return Buffer.concat(chunks);
  } finally {
    reader.releaseLock();
  }
}

// Modern requests carry capabilities in the SDK envelope. Legacy Apps clients
// use SDK-managed sessions, bounded in number/lifetime and bound to an already
// authenticated grant. A session ID never selects account authority.
export function createAuthenticatedTonkHTTP({
  oauth,
  resolveBackend,
  registerUI,
  now = Date.now,
  sessionCapacity = 32,
  sessionLifetime = 600000,
}) {
  const handlers = new Set(),
    sessions = new Map();
  let closed = false;
  const drop = async (id) => {
    const session = sessions.get(id);
    sessions.delete(id);
    await session?.server?.close();
  };
  return {
    async fetch(request) {
      if (closed) return new Response(null, { status: 503 });
      if (new URL(request.url).pathname !== '/mcp') return oauth.fetch(request);
      if (request.headers.has('origin')) return new Response(null, { status: 403 });
      const principal = oauth.authenticate(request);
      if (!principal) return oauth.challenge();
      if (request.method !== 'POST')
        return new Response(null, { status: 405, headers: { allow: 'POST' } });
      for (const [id, session] of sessions) if (session.expires <= now()) await drop(id);
      let bytes, message;
      try {
        bytes = await bounded(request.body, 200000);
      } catch {
        return new Response(null, { status: 413 });
      }
      try {
        message = JSON.parse(Buffer.from(bytes).toString('utf8'));
      } catch {
        return new Response(null, { status: 400 });
      }
      request = new Request(request.url, {
        method: 'POST',
        headers: request.headers,
        body: bytes,
        signal: request.signal,
      });
      const legacy = await isLegacyRequest(request, message);
      const sessionId = request.headers.get('mcp-session-id');
      let handler,
        retained = false,
        newId;
      if (sessionId) {
        const session = sessions.get(sessionId);
        if (!legacy || !session || session.binding !== binding(principal))
          return new Response(null, { status: 404 });
        handler = session.transport;
        retained = true;
      } else if (
        legacy &&
        message.method === 'initialize' &&
        registerUI &&
        supportsApps(message.params?.capabilities)
      ) {
        if (sessions.size >= sessionCapacity) return new Response(null, { status: 503 });
        newId = randomUUID();
        const session = {
          binding: binding(principal),
          expires: Math.min(now() + sessionLifetime, principal.connectionExpiresAt ?? Infinity),
        };
        sessions.set(newId, session); // Reserve before resolving the backend.
        try {
          const backend = await resolveBackend(principal);
          if (closed) {
            await drop(newId);
            return new Response(null, { status: 503 });
          }
          session.server = createTonkServer(backend, {
            registerUI: (server) => registerUI(server, backend),
          });
          session.transport = new WebStandardStreamableHTTPServerTransport({
            sessionIdGenerator: () => newId,
            maxRequestBodySize: 200000,
          });
          await session.server.connect(session.transport);
          handler = session.transport;
          retained = true;
        } catch (error) {
          await drop(newId);
          throw error;
        }
      } else {
        const ui =
          !legacy && supportsApps(message.params?._meta?.[CLIENT_CAPABILITIES_META_KEY])
            ? registerUI
            : undefined;
        handler = createTonkHTTP(() => resolveBackend(principal), { registerUI: ui });
        handlers.add(handler);
      }
      try {
        const response = retained
          ? await handler.handleRequest(request, { parsedBody: message })
          : await handler.fetch(request);
        const output = await bounded(response.body, 4 * 1024 * 1024);
        if (newId && !response.ok) await drop(newId);
        return new Response(response.status === 204 || response.status === 304 ? null : output, {
          status: response.status,
          headers: response.headers,
        });
      } catch (error) {
        if (newId) await drop(newId);
        // Never replay an evaluate call after an uncertain or oversized reply.
        return new Response(null, { status: 502 });
      } finally {
        if (!retained) {
          handlers.delete(handler);
          await handler.close();
        }
      }
    },
    async close() {
      closed = true;
      oauth.close();
      await Promise.allSettled(
        [...handlers].map((handler) => handler.close()).concat([...sessions.keys()].map(drop)),
      );
      handlers.clear();
    },
  };
}

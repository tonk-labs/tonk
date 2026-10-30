// The part of an LLM host that does OAuth: dynamic registration, PKCE,
// and a loopback callback the browser is sent back to.

import { createServer } from 'node:http';
import { once } from 'node:events';
import { randomBytes } from 'node:crypto';

/** A callback listener on 127.0.0.1 that resolves with the first query it sees. */
export async function loopback() {
  let resolve;
  const answered = new Promise((done) => (resolve = done));
  const server = createServer((request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    if (url.pathname !== '/callback') {
      response.writeHead(404).end();
      return;
    }
    response.writeHead(200, { 'content-type': 'text/plain' }).end('You can close this window.');
    resolve(url.searchParams);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return {
    url: `http://127.0.0.1:${server.address().port}/callback`,
    answered,
    close: () => new Promise((done) => server.close(done)),
  };
}

/**
 * An in-memory `OAuthClientProvider`. Navigation is handed to `navigate`,
 * which plays the person's browser.
 */
export function provider({ redirectUrl, name, navigate }) {
  let client, tokens, verifier, discovery;
  return {
    get redirectUrl() {
      return redirectUrl;
    },
    get clientMetadata() {
      return {
        client_name: name,
        redirect_uris: [redirectUrl],
        grant_types: ['authorization_code'],
        response_types: ['code'],
        token_endpoint_auth_method: 'none',
      };
    },
    // Claude and ChatGPT send one; the SDK sends none unless asked.
    state: () => randomBytes(16).toString('base64url'),
    clientInformation: () => client,
    saveClientInformation: (information) => (client = information),
    tokens: () => tokens,
    saveTokens: (saved) => (tokens = saved),
    redirectToAuthorization: (url) => navigate(url),
    saveCodeVerifier: (saved) => (verifier = saved),
    codeVerifier: () => verifier,
    saveDiscoveryState: (state) => (discovery = state),
    discoveryState: () => discovery,
  };
}

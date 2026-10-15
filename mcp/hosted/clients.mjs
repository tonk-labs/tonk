import { isIP } from 'node:net';
// Registrations identify redirect policy, not trusted software. Public clients
// still need user approval and S256 PKCE for every new grant.
export function createClientRegistry(clients) {
  if (!Array.isArray(clients) || !clients.length || clients.length > 64)
    throw Error('Invalid client registry');
  const entries = new Map();
  for (const client of clients) {
    if (
      typeof client.clientId !== 'string' ||
      !client.clientId ||
      client.clientId.length > 256 ||
      entries.has(client.clientId) ||
      typeof client.displayName !== 'string' ||
      !client.displayName ||
      client.displayName.length > 128 ||
      client.type !== 'public' ||
      typeof client.enabled !== 'boolean' ||
      !['web', 'native'].includes(client.applicationType) ||
      !Array.isArray(client.redirectUris) ||
      !client.redirectUris.length ||
      client.redirectUris.length > 16
    )
      throw Error('Invalid client registration');
    for (const uri of client.redirectUris) validateRedirect(uri, client.applicationType);
    entries.set(
      client.clientId,
      Object.freeze({ ...client, redirectUris: Object.freeze([...client.redirectUris]) }),
    );
  }
  return {
    get(id) {
      const client = entries.get(id);
      return client?.enabled ? client : undefined;
    },
    accepts(id, uri) {
      const client = this.get(id);
      if (!client) return false;
      try {
        validateRedirect(uri, client.applicationType);
      } catch {
        return false;
      }
      return client.redirectUris.some((registered) => {
        if (registered === uri) return true;
        const a = new URL(registered),
          b = new URL(uri);
        if (client.applicationType !== 'native' || a.protocol !== 'http:' || b.protocol !== 'http:')
          return false;
        a.port = '';
        b.port = '';
        return a.href === b.href;
      });
    },
  };
}
function validateRedirect(uri, applicationType) {
  if (typeof uri !== 'string' || uri.length > 2048) throw Error('Invalid redirect');
  const url = new URL(uri);
  if (url.username || url.password || uri.includes('#') || url.href !== uri)
    throw Error('Invalid redirect');
  if (url.protocol === 'https:') {
    const host = url.hostname;
    // Remote callbacks use DNS HTTPS names. Reject IP/private-name forms;
    // no server-side DNS lookup or metadata fetching is performed here.
    if (
      isIP(host) ||
      host.startsWith('[') ||
      !host.includes('.') ||
      host.endsWith('.') ||
      /\.(?:localhost|local|internal|home|home\.arpa)$/.test(host)
    )
      throw Error('Invalid redirect');
    return;
  }
  // Literal addresses only; do not allow URL parser aliases such as 127.1.
  if (
    applicationType === 'native' &&
    url.protocol === 'http:' &&
    ['127.0.0.1', '[::1]'].includes(url.hostname)
  )
    return;
  throw Error('Invalid redirect');
}

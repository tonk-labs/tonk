import { createHash, randomBytes } from 'node:crypto';
import { createClientRegistry } from './clients.mjs';
const random = () => randomBytes(32).toString('base64url');
const hash = (value) => createHash('sha256').update(value).digest('hex');

// Global enrollment quota survives restart. No caller-controlled forwarding/IP
// header is trusted for admission. Registration never creates account authority.
export function createRegistrations({
  now = Date.now,
  saved,
  capacity = 32,
  lifetime = 86400000,
} = {}) {
  const entries = new Map(saved?.entries ?? []);
  let enrollments = saved?.enrollments ?? [];
  if (entries.size > capacity || enrollments.length > 8)
    throw Error('Invalid registration snapshot');
  for (const [id, entry] of entries) {
    if (
      id !== entry.client.clientId ||
      !Number.isFinite(entry.expires) ||
      !/^[a-f0-9]{64}$/.test(entry.managementHash)
    )
      throw Error('Invalid registration snapshot');
    createClientRegistry([entry.client]);
  }
  const prune = () => {
    for (const [id, entry] of entries) if (entry.expires <= now()) entries.delete(id);
    enrollments = enrollments.filter((time) => time > now() - 60000);
  };
  return {
    get(id) {
      prune();
      return entries.get(id)?.client;
    },
    register(metadata) {
      prune();
      if (entries.size >= capacity || enrollments.length >= 8)
        return { error: 'temporarily_unavailable', status: 429 };
      // Real clients send more metadata than this server uses (logo, contacts,
      // client_uri...). Ignore what is not read; refuse only what would change
      // how the client authenticates or which grants it uses.
      const grants = metadata?.grant_types ?? ['authorization_code', 'refresh_token'];
      if (
        !metadata ||
        typeof metadata !== 'object' ||
        Array.isArray(metadata) ||
        ![undefined, 'none'].includes(metadata.token_endpoint_auth_method) ||
        !Array.isArray(grants) ||
        !grants.includes('authorization_code') ||
        grants.some((grant) => !['authorization_code', 'refresh_token'].includes(grant)) ||
        (metadata.response_types !== undefined &&
          JSON.stringify(metadata.response_types) !== '["code"]') ||
        (metadata.client_name !== undefined && typeof metadata.client_name !== 'string')
      )
        return { error: 'invalid_client_metadata', status: 400 };
      const client = {
        clientId: 'dynamic-' + random(),
        displayName: metadata.client_name?.trim().slice(0, 128) || 'MCP client',
        redirectUris: metadata.redirect_uris,
        applicationType: metadata.application_type ?? 'web',
        type: 'public',
        enabled: true,
      };
      try {
        createClientRegistry([client]);
      } catch {
        return { error: 'invalid_client_metadata', status: 400 };
      }
      const managementToken = random();
      entries.set(client.clientId, {
        client,
        managementHash: hash(managementToken),
        expires: now() + lifetime,
      });
      enrollments.push(now());
      return { client, managementToken, expires: now() + lifetime };
    },
    remove(id, token) {
      prune();
      const entry = entries.get(id);
      if (
        !entry ||
        typeof token !== 'string' ||
        !/^[A-Za-z0-9_-]{43}$/.test(token) ||
        entry.managementHash !== hash(token)
      )
        return false;
      entries.delete(id);
      return true;
    },
    snapshot() {
      prune();
      return { entries: [...entries], enrollments: [...enrollments] };
    },
  };
}

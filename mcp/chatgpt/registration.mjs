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
      if (
        !metadata ||
        typeof metadata !== 'object' ||
        Array.isArray(metadata) ||
        Object.keys(metadata).some(
          (key) =>
            ![
              'client_name',
              'redirect_uris',
              'application_type',
              'token_endpoint_auth_method',
              'grant_types',
              'response_types',
              'scope',
            ].includes(key),
        ) ||
        metadata.token_endpoint_auth_method !== 'none' ||
        (metadata.scope !== undefined && metadata.scope !== 'tonk') ||
        (metadata.grant_types !== undefined &&
          JSON.stringify(metadata.grant_types) !==
            JSON.stringify(['authorization_code', 'refresh_token'])) ||
        (metadata.response_types !== undefined &&
          JSON.stringify(metadata.response_types) !== '["code"]')
      )
        return { error: 'invalid_client_metadata', status: 400 };
      const client = {
        clientId: 'dynamic-' + random(),
        displayName: metadata.client_name,
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

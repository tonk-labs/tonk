import { lstat, realpath } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { startNativeAccount } from './native.mjs';

// Host-owned bindings, not model arguments: chooseSubject must consult the
// user's explicit space selection. One child pins one tenant to one space.
// Outbound network policy must be installed before enabling this resolver.
export function createTenantBackends({ binary, dataRoot, chooseSubject, capacity = 16,
  startAccount = startNativeAccount }) {
  if (!Number.isSafeInteger(capacity) || capacity < 1) throw new Error('Invalid tenant capacity.');
  const tenants = new Map();
  let closed = false;
  async function open(principal, subject) {
    const root = await realpath(dataRoot);
    const directory = join(root, principal.tenantId);
    if ((await lstat(directory)).isSymbolicLink() || dirname(await realpath(directory)) !== root) {
      throw new Error('Invalid tenant directory.');
    }
    const account = await startAccount({ binary, dataDirectory: directory, enableSpaces: true });
    try {
      const identity = await account.status();
      if (closed || identity.deviceDid !== principal.deviceDid || identity.rootDid !== principal.rootDid) {
        throw new Error('OAuth principal does not match the native account.');
      }
      return await account.openSpace(subject);
    } catch (error) { await account.close(); throw error; }
  }
  return {
    async resolve(principal) {
      if (closed) throw new Error('Tenant backends are closed.');
      if (!/^tenant-[A-Za-z0-9]{6}$/.test(principal.tenantId)) throw new Error('Invalid tenant identity.');
      const subject = await chooseSubject(Object.freeze({ ...principal }));
      if (closed) throw new Error('Tenant backends are closed.');
      if (typeof subject !== 'string' || !subject.startsWith('did:key:')) throw new Error('Select an account space first.');
      let entry = tenants.get(principal.tenantId);
      if (entry) {
        if (entry.deviceDid !== principal.deviceDid || entry.rootDid !== principal.rootDid || entry.subject !== subject) {
          throw new Error('This tenant is already pinned to another identity or space.');
        }
        return entry.backend;
      }
      if (tenants.size >= capacity) throw new Error('Hosted runtime capacity reached.');
      entry = { deviceDid: principal.deviceDid, rootDid: principal.rootDid, subject };
      tenants.set(principal.tenantId, entry);
      entry.backend = open(principal, subject).catch(error => {
        if (tenants.get(principal.tenantId) === entry) tenants.delete(principal.tenantId);
        throw error;
      });
      return entry.backend;
    },
    async close() {
      closed = true;
      const pending = [...tenants.values()].map(entry => entry.backend);
      tenants.clear();
      const results = await Promise.allSettled(pending.map(async backend => (await backend).close()));
      if (results.some(result => result.status === 'rejected')) throw new Error('Could not close every tenant runtime.');
    },
  };
}

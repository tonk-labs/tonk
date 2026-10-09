import { mkdtemp } from 'node:fs/promises';
import { basename, join } from 'node:path';
import { startNativeAccount } from './native.mjs';

// The host supplies a private root; the browser never supplies a tenant path.
// Keep directories after an uncertain authorization. A failed HTTP exchange is
// not permission to erase an identity that Tonk may already have attached.
export function nativeAccountProvisioner({ binary, dataRoot }) {
  return async () => {
    const directory = await mkdtemp(join(dataRoot, 'tenant-'));
    const account = await startNativeAccount({ binary, dataDirectory: directory });
    const deviceDid = account.deviceDid;
    await account.close();
    return {
      tenantId: basename(directory), deviceDid,
      async authorize(authorization) {
        const runtime = await startNativeAccount({ binary, dataDirectory: directory });
        try {
          if (runtime.deviceDid !== deviceDid) throw new Error('Tenant identity changed.');
          return await runtime.authorize(authorization);
        } finally { await runtime.close(); }
      },
    };
  };
}

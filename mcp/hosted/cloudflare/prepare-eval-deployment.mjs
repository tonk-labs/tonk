import { appendFile, writeFile } from 'node:fs/promises';
import { evalDeployment } from './eval-deployment.mjs';

const required = (name) => {
  const value = process.env[name];
  if (!value) throw Error(`${name} is required`);
  return value;
};

const accountId = required('CLOUDFLARE_ACCOUNT_ID');
const token = required('CLOUDFLARE_API_TOKEN');
const config = evalDeployment({
  suffix: required('EVAL_SUFFIX'),
  provider: required('TONK_PROVIDER_ORIGIN'),
  subdomain: required('CLOUDFLARE_SUBDOMAIN'),
  accountId,
  commit: required('BUILD_COMMIT'),
});

async function api(path, options = {}) {
  const response = await fetch(
    `https://api.cloudflare.com/client/v4/accounts/${accountId}/${path}`,
    {
      ...options,
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      signal: AbortSignal.timeout(30000),
      redirect: 'error',
    },
  );
  if (response.status === 404) return undefined;
  const body = await response.json().catch(() => undefined);
  if (!response.ok || !body?.success)
    throw Error(`Cloudflare ${path} failed (${response.status}); refusing to infer absence`);
  return body.result;
}

// Check every collision before creating state. A pre-existing resource is never
// reused or overwritten. If bucket creation later succeeds but deployment fails,
// it stays isolated for explicit operator inspection rather than being deleted.
const bucket = config.r2_buckets[0].bucket_name;
const existingWorker = await api(`workers/scripts/${config.name}`);
if (existingWorker !== undefined) throw Error(`Evaluation worker collision: ${config.name}`);
const existingBucket = await api(`r2/buckets/${bucket}`);
if (existingBucket !== undefined) throw Error(`Evaluation checkpoint bucket collision: ${bucket}`);
await api('r2/buckets', {
  method: 'POST',
  body: JSON.stringify({ name: bucket, locationHint: 'weur' }),
});

await writeFile(
  new URL('./wrangler.eval.generated.json', import.meta.url),
  JSON.stringify(config, null, 2) + '\n',
);
await writeFile(
  new URL('../build.json', import.meta.url),
  JSON.stringify({ commit: config.vars.TONK_BUILD_SHA }) + '\n',
);
if (process.env.GITHUB_OUTPUT) {
  await appendFile(
    process.env.GITHUB_OUTPUT,
    `issuer=${config.vars.TONK_MCP_ISSUER}\nworker=${config.name}\nbucket=${bucket}\n`,
  );
}
console.log(`Prepared new evaluation worker ${config.name} for ${config.vars.TONK_MCP_ISSUER}`);

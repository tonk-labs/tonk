import { writeFile, appendFile } from 'node:fs/promises';
import { deployment } from './deployment.mjs';

const accountId = process.env.CLOUDFLARE_ACCOUNT_ID;
const token = process.env.CLOUDFLARE_API_TOKEN;
if (!accountId || !token) throw Error('Cloudflare deployment credentials are required');
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
  const body = await response.json();
  if (response.status === 404) return undefined;
  if (!response.ok || !body.success) throw Error(`Cloudflare ${path} failed (${response.status})`);
  return body.result;
}
const namespace = await api('workers/subdomain');
const config = deployment({
  channel: process.env.DEPLOY_CHANNEL,
  pullRequest: process.env.PULL_REQUEST,
  provider: process.env.TONK_PROVIDER_ORIGIN,
  subdomain: namespace.subdomain,
  accountId,
  commit: process.env.BUILD_COMMIT,
});
const bucket = config.r2_buckets[0].bucket_name;
if (!(await api(`r2/buckets/${bucket}`))) {
  await api('r2/buckets', {
    method: 'POST',
    body: JSON.stringify({ name: bucket, locationHint: 'weur' }),
  });
}
await writeFile(
  new URL('./wrangler.generated.json', import.meta.url),
  JSON.stringify(config, null, 2) + '\n',
);
if (process.env.GITHUB_OUTPUT) {
  await appendFile(
    process.env.GITHUB_OUTPUT,
    `url=${config.vars.TONK_MCP_ISSUER}/mcp\nissuer=${config.vars.TONK_MCP_ISSUER}\nclient_id=${config.vars.TONK_MCP_CLIENT_ID}\ncallback=${config.vars.TONK_MCP_REDIRECT_URI}\n`,
  );
}
console.log(`Prepared ${config.name}: ${config.vars.TONK_MCP_ISSUER}/mcp`);

// Bake identity into the image so smoke cannot mistake an old container for this build.
await writeFile(
  new URL('../build.json', import.meta.url),
  JSON.stringify({ commit: config.vars.TONK_BUILD_SHA }),
);

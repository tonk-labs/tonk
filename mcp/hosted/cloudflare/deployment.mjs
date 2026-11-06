import { providerOrigin } from '../provider.mjs';

export function deployment({ channel, pullRequest, provider, subdomain, accountId, commit }) {
  if (!['preview', 'dev', 'staging', 'stable'].includes(channel))
    throw Error('Invalid deployment channel');
  if (!/^[a-z0-9-]+$/.test(subdomain)) throw Error('Invalid workers.dev subdomain');
  if (!/^[a-f0-9]{32}$/.test(accountId)) throw Error('Invalid Cloudflare account');
  if (!/^[a-f0-9]{40}$/.test(commit)) throw Error('Invalid build commit');
  if (channel === 'preview' && !/^[1-9][0-9]*$/.test(String(pullRequest)))
    throw Error('Invalid PR number');
  const suffix = channel === 'preview' ? `pr-${pullRequest}` : channel;
  const name = channel === 'stable' ? 'tonk-mcp' : `tonk-mcp-${suffix}`;
  // Release channels get one-level tonk.host names (the zone has no nested
  // subdomains). PR previews stay on workers.dev so they leave no DNS behind.
  const domain = {
    dev: 'agent-dev.tonk.host',
    staging: 'agent-staging.tonk.host',
    stable: 'agent.tonk.host',
  }[channel];
  const issuer = `https://${domain ?? `${name}.${subdomain}.workers.dev`}`;
  const expectedProvider = {
    dev: 'https://tonk.foundation',
    staging: 'https://staging.tonk.xyz',
    stable: 'https://tonk.network',
  }[channel];
  provider = providerOrigin(provider);
  if (expectedProvider && provider !== expectedProvider)
    throw Error('Provider does not match release channel');
  if (
    channel === 'preview' &&
    provider !== `https://tonk-access-service-pr-${pullRequest}.${subdomain}.workers.dev`
  )
    throw Error('Provider does not match PR preview');
  return {
    name,
    main: 'worker.mjs',
    compatibility_date: '2026-10-07',
    account_id: accountId,
    // The issuer is the only origin the Worker answers (others get 421), so a
    // channel with a domain does not also expose a workers.dev URL.
    workers_dev: !domain,
    routes: domain ? [{ pattern: domain, custom_domain: true }] : [],
    containers: [
      {
        class_name: 'TonkRuntime',
        image: './Dockerfile.worker',
        image_build_context: '../../..',
        max_instances: 1,
        instance_type: 'basic',
      },
    ],
    durable_objects: { bindings: [{ name: 'RUNTIME', class_name: 'TonkRuntime' }] },
    migrations: [{ tag: 'v1', new_sqlite_classes: ['TonkRuntime'] }],
    r2_buckets: [{ binding: 'CHECKPOINTS', bucket_name: `${name}-checkpoints` }],
    observability: { enabled: false },
    assets: {
      directory: '../public',
      binding: 'ASSETS',
      run_worker_first: true,
      html_handling: 'none',
    },
    vars: {
      TONK_MCP_ISSUER: issuer,
      TONK_PROVIDER_ORIGIN: provider,
      TONK_BUILD_SHA: commit,
    },
  };
}

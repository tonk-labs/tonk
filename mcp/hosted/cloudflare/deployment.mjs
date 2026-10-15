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
  const issuer = `https://${name}.${subdomain}.workers.dev`;
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
    provider !== `https://pr-${pullRequest}-tonk-access-service-preview.${subdomain}.workers.dev`
  )
    throw Error('Provider does not match PR preview');
  return {
    name,
    main: 'worker.mjs',
    compatibility_date: '2026-10-07',
    account_id: accountId,
    workers_dev: true,
    routes: [],
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
      TONK_MCP_SPACE: name,
      TONK_PROVIDER_ORIGIN: provider,
      TONK_BUILD_SHA: commit,
      TONK_MCP_CLIENT_ID: `${name}-chatgpt`,
      TONK_MCP_REDIRECT_URI: 'https://chatgpt.com/connector_platform_oauth_redirect',
      TONK_MCP_CLIENTS: JSON.stringify([
        {
          clientId: `${name}-chatgpt`,
          displayName: 'Tonk for ChatGPT',
          type: 'public',
          applicationType: 'web',
          enabled: true,
          redirectUris: ['https://chatgpt.com/connector_platform_oauth_redirect'],
        },
        {
          clientId: `${name}-claude`,
          displayName: 'Tonk for Claude',
          type: 'public',
          applicationType: 'web',
          enabled: true,
          redirectUris: ['https://claude.ai/api/mcp/auth_callback'],
        },
        {
          clientId: `${name}-native`,
          displayName: 'Tonk native client',
          type: 'public',
          applicationType: 'native',
          enabled: true,
          redirectUris: ['http://127.0.0.1/callback', 'http://[::1]/callback'],
        },
      ]),
    },
  };
}

import { providerOrigin } from '../provider.mjs';

const suffixPattern = /^eval-[a-z0-9][a-z0-9-]{4,29}[a-z0-9]$/;

export function evalDeployment({ suffix, provider, subdomain, accountId, commit }) {
  if (!suffixPattern.test(suffix)) throw Error('Invalid fresh evaluation suffix');
  if (providerOrigin(provider) !== 'https://tonk.foundation')
    throw Error('Evaluation deployment requires the trusted foundation provider');
  if (!/^[a-z0-9-]+$/.test(subdomain)) throw Error('Invalid workers.dev subdomain');
  if (!/^[a-f0-9]{32}$/.test(accountId)) throw Error('Invalid Cloudflare account');
  if (!/^[a-f0-9]{40}$/.test(commit)) throw Error('Invalid build commit');

  const name = `tonk-mcp-${suffix}`;
  const issuer = `https://${name}.${subdomain}.workers.dev`;
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
      TONK_PROVIDER_ORIGIN: 'https://tonk.foundation',
      TONK_BUILD_SHA: commit,
    },
  };
}

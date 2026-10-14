# Cloudflare MCP deployment

The repository Deploy workflow builds the matching frontend assets, deploys Tonk,
and then deploys MCP on a fresh Docker runner. The generated Wrangler config is
ignored. Deployment requires the existing `CLOUDFLARE_ACCOUNT_ID` and
`CLOUDFLARE_API_TOKEN` secrets, with Workers, Containers and R2 permissions.

| Trigger | MCP worker | Tonk provider |
| --- | --- | --- |
| Same-repository PR | `tonk-mcp-pr-N` | Matching `pr-N` access preview |
| Push to main | `tonk-mcp-dev` | `https://tonk.foundation` |
| Prerelease v tag | `tonk-mcp-staging` | `https://staging.tonk.xyz` |
| Final v tag | `tonk-mcp` | `https://tonk.network` |

Each worker has its own workers.dev issuer, `${worker}-chatgpt` public OAuth
client and private `${worker}-checkpoints` bucket. The callback is
`https://chatgpt.com/connector_platform_oauth_redirect`. CI records connection
settings in the job summary and PR comment after smoke checks pass. Those checks
exercise container-backed OAuth discovery, the unauthenticated MCP challenge,
public assets and the deployed commit. They do not complete ChatGPT linking.

Preview checkpoints are retained; closing a PR does not discard unpublished
replica data or credentials. Preview lifecycle cleanup remains an operator task.
Release channel parity does not establish production durability or capacity;
see [the adapter readiness limits](../README.md).

## Local packaging

The checked-in `wrangler.toml` is the separate `tonk-mcp-test` development host,
using `tonk.foundation`. CI uses `prepare-deployment.mjs` instead. From the root:

```sh
npm --prefix mcp ci --ignore-scripts
npm --prefix mcp/hosted/cloudflare ci
TONK_UI_DIST=/absolute/path/to/tonk-ui npm --prefix mcp run build:chatgpt-assets
docker build --platform linux/amd64 -f mcp/hosted/cloudflare/Dockerfile.worker -t tonk-chatgpt:test .
cd mcp/hosted/cloudflare
npx --no-install wrangler deploy --dry-run --containers-rollout=none --config wrangler.toml
```

The dry run validates the Worker bundle, not the container or public deployment.
The image runs the ordinary `tonk-worker-host` binary and `hosted/server.mjs`.
Its entrypoint installs the Cloudflare interception CA. For manual deployment,
provision the test checkpoint bucket and set `TONK_MCP_ISSUER`, `TONK_MCP_SPACE`
(deployment namespace), `TONK_MCP_CLIENT_ID` and `TONK_MCP_REDIRECT_URI` through
Wrangler secrets. `TONK_PROVIDER_ORIGIN` is the trusted configured sync provider.
Changing these values requires restarting the container.

## Storage and network

The public Worker chooses one named container; callers cannot choose instances.
Outbound traffic is limited to the configured Tonk provider and the internal checkpoint proxy,
with redirects not followed. Closed authorization seeds use private R2 conditional
checkpoints; credential state has a separate conditional object. Checkpoints are
bounded to 64 MiB compressed, 256 MiB expanded and 4096 files.

Live workers use separate local copies of those seeds and publish space writes
through normal sync. Restarting a child process preserves its same local directory
and verifies the account binding. Container replacement only has the seed and
remote data: interrupted or unpublished effects need an explicit durability policy
before production use. The 20-minute container sleep and 16-worker capacity are
development settings, not a verified multi-user lifecycle policy.

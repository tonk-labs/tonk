# Development Cloudflare host

The checked-in configuration is the existing `tonk-mcp-test` development host.
It uses `tonk.foundation` for approval and synchronization. Use this config,
not the repository-root Wrangler configuration. No deployment is performed by CI.

From the repository root:

```sh
npm --prefix mcp ci --ignore-scripts
npm --prefix mcp/chatgpt/cloudflare ci --ignore-scripts
npm --prefix mcp run build:chatgpt-assets
docker build --platform linux/amd64 -f mcp/chatgpt/cloudflare/Dockerfile.worker -t tonk-chatgpt:test .
wrangler deploy --dry-run --containers-rollout=none --config mcp/chatgpt/cloudflare/wrangler.toml
```

The dry run validates the Worker bundle, not the container or public deployment.
The image uses the merged ordinary `tonk-worker-host` binary and starts
`chatgpt/server.mjs`. The entrypoint installs the Cloudflare interception CA;
a local image smoke test must bypass that platform-specific entrypoint.

## Deployment configuration

The development checkpoint bucket is private `tonk-mcp-test-checkpoints`.
Provision it separately if setting up a new host. Configure these through
`wrangler secret put NAME --config mcp/chatgpt/cloudflare/wrangler.toml`:

- `TONK_MCP_ISSUER`: deployed HTTPS origin, without a trailing slash.
- `TONK_MCP_SPACE`: existing deployment/checkpoint namespace identifier. This
  compatibility field does not select the account or tool target.
- `TONK_MCP_CLIENT_ID`: exact predefined public OAuth client.
- `TONK_MCP_REDIRECT_URI`: exact callback registered for that client.

The MCP URL is the issuer plus `/mcp`. A configured issuer alone serves discovery
and the authentication challenge; the host starts only when all settings exist.
Changing configuration requires restarting the container. Deployment is an
explicit operator step after the acceptance gates in [the adapter README](../README.md).

## Storage and network

The public Worker chooses one named container; callers cannot choose instances.
Outbound traffic is limited to `tonk.foundation` and the internal checkpoint proxy,
with redirects not followed. Closed authorization seeds use private R2 conditional
checkpoints; credential state has a separate conditional object. Checkpoints are
bounded to 64 MiB compressed, 256 MiB expanded and 4096 files.

Live workers use separate local copies of those seeds and publish space writes
through normal sync. Restarting a child process preserves its same local directory
and verifies the account binding. Container replacement only has the seed and
remote data: interrupted or unpublished effects need an explicit durability policy
before production use. The 20-minute container sleep and 16-worker capacity are
development settings, not a verified multi-user lifecycle policy.

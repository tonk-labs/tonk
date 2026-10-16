# Cloudflare MCP deployment

The repository Deploy workflow builds the matching frontend assets, deploys Tonk,
and then deploys MCP on a fresh Docker runner. The generated Wrangler config is
ignored. Deployment requires the existing `CLOUDFLARE_ACCOUNT_ID` and
`CLOUDFLARE_API_TOKEN` secrets, with Workers, Containers and R2 permissions.

| Trigger | MCP worker | MCP URL | Tonk provider |
| --- | --- | --- | --- |
| Same-repository PR | `tonk-mcp-pr-N` | `https://tonk-mcp-pr-N.<subdomain>.workers.dev/mcp` | Matching `pr-N` access preview |
| Push to main | `tonk-mcp-dev` | `https://agent-dev.tonk.host/mcp` | `https://tonk.foundation` |
| Prerelease v tag | `tonk-mcp-staging` | `https://agent-staging.tonk.host/mcp` | `https://staging.tonk.xyz` |
| Final v tag | `tonk-mcp` | `https://agent.tonk.host/mcp` | `https://tonk.network` |

Each worker has its own origin and private checkpoint bucket. Release channels
are served only from their `tonk.host` custom domain (one-level names; the zone
has no nested subdomains), with workers.dev disabled because the Worker answers
only its issuer origin. PR previews stay on workers.dev so they leave no DNS
records behind. Custom domains need the deploy token to hold Workers Routes and
DNS edit permissions on the `tonk.host` zone. Configure
clients for Streamable HTTP without OAuth; every query/evaluation supplies an
agent link. CI smoke checks the deployed commit, public tool discovery, required
link fields and rejection of missing links. It does not exercise real user data.

Preview checkpoints are retained. Closing a PR does not discard unpublished data.
See [agent-link lifecycle and recovery](../../AGENT-LINKS.md) before operating it.

## Local packaging

The checked-in `wrangler.toml` is the separate `tonk-mcp-test` development host,
using `tonk.foundation`. CI uses `prepare-deployment.mjs` instead. From the root:

```sh
npm --prefix mcp ci --ignore-scripts
npm --prefix mcp/hosted/cloudflare ci
TONK_UI_DIST=/absolute/path/to/tonk-ui npm --prefix mcp run build:hosted-assets
docker build --platform linux/amd64 -f mcp/hosted/cloudflare/Dockerfile.worker -t tonk-chatgpt:test .
cd mcp/hosted/cloudflare
npx --no-install wrangler deploy --dry-run --containers-rollout=none --config wrangler.toml
```

The dry run validates the Worker bundle, not the container or public deployment.
The image runs `tonk-agent-worker` and `hosted/server.mjs`. Its entrypoint installs
the Cloudflare interception CA. Configure `TONK_MCP_ISSUER` and the trusted
`TONK_PROVIDER_ORIGIN`; OAuth client registrations are no longer required.

## Storage and network

The public Worker selects the `agent-link-v1` container. Egress is restricted to
the pinned provider and internal checkpoint proxy. Agent replicas use a separate
`agent-state-v1.json` conditional object; old OAuth state is retained unused.
Checkpoint limits are 64 MiB compressed, 256 MiB expanded and 4096 files. The
native worker has lower admission limits to leave checkpoint headroom. Calls
are serialized and closed workers checkpoint before acknowledgment, including
error outcomes. Container loss during an invocation remains uncertain; see the
agent-link guide for precise durability limits and operator recovery.

# Cloudflare hosted ChatGPT test

This is a separate `tonk-mcp-test` Worker and native container. It uses
`https://tonk.foundation` for Tonk approval and synchronization. It does not
deploy or change the existing Tonk service. No desktop runtime participates.

## Before deployment

1. Have a working Docker-compatible builder (`docker info` must succeed).
2. Select a disposable space on `tonk.foundation` and record its exact DID.
3. Install this directory's pinned dependency with `npm ci --ignore-scripts`.
4. Build and test the Linux/amd64 image. From the repository root:

   ```sh
   docker build --platform linux/amd64 -f mcp/cloudflare/Dockerfile.worker -t tonk-mcp-test .
   ```

5. Create the **private** checkpoint bucket once:

   ```sh
   wrangler r2 bucket create tonk-mcp-test-checkpoints
   ```

6. Deploy using this configuration, never the repository's root Wrangler file:

   ```sh
   wrangler deploy --config mcp/cloudflare/wrangler.toml
   ```

The Worker returns 503 until its issuer is set. With the issuer configured,
registration mode exposes OAuth discovery and an MCP 401 challenge, but does
not start a container or authorize access until all four settings are present.
Use the deployed HTTPS origin as `TONK_MCP_ISSUER`, without a trailing slash;
the MCP URL is that origin plus `/mcp`. Register a predefined public OAuth
client in ChatGPT and copy its **exact** redirect URI from the connection UI.
Set these with `wrangler secret put NAME --config mcp/cloudflare/wrangler.toml`:

- `TONK_MCP_ISSUER`
- `TONK_MCP_SPACE` (legacy deployment namespace DID; tools select account spaces explicitly)
- `TONK_MCP_CLIENT_ID`
- `TONK_MCP_REDIRECT_URI`

Re-deploy/restart the container after changing configuration; it receives the
configuration when it starts. Never include credentials in command arguments,
checked-in configuration, logs or test output.

## State and network boundaries

- One named container hosts bounded, isolated full-worker instances. Frontend
  query subscriptions stream directly through `/space-api/`; they do not enter
  the MCP tool queue or checkpoint after reads.
- OAuth creates a private worker identity, imports the approved root grant and
  completes account hydration. Its closed authorization seed is checkpointed to
  private R2 with conditional replacement. Live replicas are separate copies.
- Successful evaluate/transact requests publish once through ordinary worker
  sync. Failed publication reports an uncertain local change and never replays it.
  Live caches are not checkpointed while open; acknowledged changes must be
  recoverable from the Tonk remote. Verify this with a real account before release.
- OAuth and browser access credentials last ten minutes and rotate using refresh
  credentials within a fixed eight-hour connection window. Browser renewal
  preserves the mounted space and subscriptions without replaying edits. State
  is saved separately in private R2 before credential responses are returned.
  Restarts restore issued credentials; incomplete approval still starts over.
  A bounded, nonrotating reopening grant in private tool metadata lets a cached
  widget obtain fresh sessions within the original connection deadline. It
  cannot select another space or extend authorization. Refresh credentials
  still rotate and reject replay. Reopening creates a separate session so
  another mounted copy remains valid. Admission remains capped at 64 browser
  credential families and 64 reopening grants per instance.
  Network-interrupted edits are never replayed; reload to inspect saved content.
  Each browser session is restricted to one selected space/main branch;
  credentials stay out of model-visible tool output and guest-frame context.
- The checkpoint is bounded to 64 MiB compressed, 256 MiB expanded JSON and 4096 files. Recovery saves retain only active OAuth/widget authorities and pending approvals.
  Expired replica directories are omitted from checkpoints but remain on local
  disk until replacement; local cleanup and multi-user operations remain follow-up work.
- Internet access is off except `tonk.foundation` and the private checkpoint
  proxy. Account/profile endpoints are never exposed to the embedded space.

## Evidence and remaining gates

Local tests cover fresh-directory native recovery, stale checkpoint rejection,
failure before acknowledgement, outbound request filtering, and push failure
without repeated evaluation. Worker bundling can be checked without Docker:

```sh
wrangler deploy --dry-run --containers-rollout=none --config mcp/cloudflare/wrangler.toml
```

That command does **not** validate the Linux image, Cloudflare interception,
real R2 conditional writes or deployment. Before declaring the slice working,
exercise those boundaries, connect ChatGPT, create/query/update the reading
list, verify it in Tonk, then replace the container and verify the same OAuth
connection plus a reloaded widget without reconnecting. In-flight edits must
be inspected rather than replayed.

## Current development architecture (2026-10-08)

MCP URL: `https://tonk-mcp-test.tonk.workers.dev/mcp`.
Public OAuth client: `tonk-chatgpt-test`, no client secret.
Callback: `https://chatgpt.com/connector_platform_oauth_redirect`.

The full-worker replacement exposes query, evaluate, space info and open space.
No component-specific read/edit tools are registered. Current deployment evidence
and outstanding live gates are recorded in
`plans/2026-10-08-shared-worker-host.md` at the repository root.

### Account space selection

Call `tonk_list_spaces` to discover the connected account catalog. Hosted
`tonk_query`, `tonk_evaluate`, `tonk_space_info`, and `tonk_open_space` require
`space`, the exact returned subject. Resolve ambiguous names with the user.
There is no account-wide mutable selection. Each iframe remains scoped to
its original space, and membership is checked before worker requests.
The configured TONK_MCP_SPACE remains a deployment/checkpoint compatibility
identifier; it no longer chooses the target of hosted tool calls.

Native worker exits are recovered on the next request using the same live data
folder and verified account identity. The interrupted request is never replayed.
A read failure alone is not evidence that OAuth expired; catalog/load HTTP
failures and worker interruptions report separate safe messages. Unexpected
child exits log only exit code/signal, with no stderr or account data.

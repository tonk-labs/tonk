# tonk-host-worker

The hosted agent's `/mcp`. The access service forwards `/mcp` to this worker
over the `MCP_WORKER` service binding, so the endpoint lives on the provider's
origin, which is the OAuth resource the authorization server issues tokens for.
Writes go through one space writer Durable Object per space and MCP session (see
`src/lib.rs` and `tonk_mcp`), which drives the space's own kernel, booted in
the isolate.

## Building

A Worker compiles no WebAssembly at run time, so it is deployed with the
space's kernel bundled ahead of time, in `kernel/`: the component, its core
modules left as skeletons (every body `unreachable`), and every module the
component model polyfill makes, compiled, by the SHA-256 of its bytes.
`scripts/bundle.sh` writes it: it boots the kernel in Node, which does
compile, recording each module the polyfill makes, and files the core modules
under their skeletons' keys (`rust/tonk-kernel-bundle`). Then
`worker-build --release` builds the Worker; `entry.mjs` puts the kernel on the
global object before the Rust module loads.

```sh
./scripts/bundle.sh
worker-build --release
```

`scripts/smoke.mjs` is an MCP tool call end to end under `wrangler dev
--local`: a native access service holding an agent space with a note
(`cargo run -p tonk-mcp --features native-kernels --example
hosted_fixture`), queried, written and read back through `/mcp`.

```sh
cargo build -p tonk-mcp --features native-kernels --example hosted_fixture
node scripts/smoke.mjs ../../target/debug/examples/hosted_fixture
```

Every pull request deploys `tonk-mcp-worker-pr-<N>` and binds it into
`tonk-access-service-pr-<N>` (`.github/workflows/publish.yml`). The
`wrangler.toml` name here, `tonk-mcp-worker-local`, is for `wrangler dev`;
deployments always pass `--name`. The bot's preview
comment gives the access preview URL, called `$PREVIEW` below.

## Manual end-to-end test

### 0. Check the deployment

```sh
PREVIEW=https://tonk-access-service-pr-<N>.<subdomain>.workers.dev

# 401, with the metadata link the client follows:
curl -si -X POST "$PREVIEW/mcp" | grep -i -E '^HTTP|^www-authenticate'
# WWW-Authenticate: Bearer resource_metadata="$PREVIEW/.well-known/oauth-protected-resource/mcp"

curl -s "$PREVIEW/.well-known/oauth-protected-resource/mcp"   # resource = $PREVIEW/mcp
curl -s "$PREVIEW/.well-known/oauth-authorization-server"
```

If `/mcp` answers `404 this deployment serves no /mcp`, the binding is
missing. If it answers `500 MCP_SESSION_SECRET unset`, the deploy step that
sets the secret did not run.

### 1. Have an account and a space on the preview

Open `$PREVIEW` in a browser, create an account (passkey), and create a space
with a little content. Passkeys are per origin, so this account exists only on
this preview.

### 2a. Claude Code

```sh
claude mcp add --transport http tonk-preview "$PREVIEW/mcp"
claude
```

1. Run `/mcp`, select `tonk-preview`, and choose Authenticate. A browser opens
   `$PREVIEW/authorize`, which hands off to `/settings/agent`.
2. Sign in with the passkey. Create or pick the agent, select the space and an
   access level (read, or read and write), and approve. The browser returns to
   Claude Code's callback and `/mcp` shows the server connected with 4 tools.
3. Ask: "List my Tonk spaces." Expect `tonk_list_spaces` to return the agent
   space marked `home: true` and the granted space with its name and access level.
4. Ask: "In <space>, list the concepts." Expect `tonk_query` with that space
   subject to succeed.
5. Ask: "In <space>, add a note titled hello." Expect `tonk_evaluate` to
   succeed with a read-and-write grant, and to come back as a tool error
   (`may only read`) with a read grant.
6. Check the write in the browser: reload the space on `$PREVIEW` and find the
   record. Then ask Claude to query it back.

To start over: `claude mcp remove tonk-preview`, then add it again.

### 2b. Claude.ai

1. Settings, Connectors, Add custom connector. Name `Tonk preview`, URL
   `$PREVIEW/mcp`. Add, then Connect.
2. The same approval as in Claude Code opens in a new tab: sign in, pick the
   agent, the space and the access level, approve. The tab closes and the
   connector shows as connected.
3. In a new chat, enable the connector from the tools menu, then repeat steps
   3 to 6 of Claude Code.

### What to look for when something fails

- The preview's Workers logs (`tonk-mcp-worker-pr-<N>`, observability is on)
  show each `/mcp` request and the writer objects' errors.
- `401` after approval: the token did not verify. The body names why (a grant
  not issued by its space, a powerline addressed elsewhere).
- `404 unknown MCP session`: the session id did not verify for this token. The
  client should initialize again; if it does not, reconnect.
- A tool error with `PULL_FAILED` or `WRITE_NOT_LANDED`: the access service
  refused the writer's sync (an unprovisioned space, a revoked session or
  grant). A write answered `WRITE_NOT_LANDED` did not land.

## Local

Start `tonk-access-local` (see the `run-app` skill), build as above, and
serve with:

```sh
wrangler dev --var ACCESS_SERVICE_URL:<access url> --var MCP_SESSION_SECRET:dev
```

The automated equivalent of the manual test, natively against a local access
service with the space's kernels booted on Wasmtime, is `cargo test -p
tonk-mcp --features integration-tests`.

## Not done yet

- Revocation is not checked by the front. The access service checks every
  chain at each sync, and every query pulls and every write pulls and pushes,
  so a revoked session or grant fails at its next call. A read served while a
  write holds the lock skips the pull.
- No SSE, no assets, no read caps.
- Dev, staging and production deploy `tonk-mcp-worker-dev`,
  `tonk-mcp-worker-staging` and `tonk-mcp-worker` before the access service
  and bind them as `MCP_WORKER`; the deploy then checks that `/mcp` answers an
  unauthenticated request with 401.

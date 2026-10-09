# Tonk MCP

Canonical tool definitions and MCP server for Tonk. Tonk Town and the hosted
ChatGPT integration share the contract; **ChatGPT does not depend on the desktop
app**. Runtime adapters supply authorized capabilities and execute tools. The
MCP SDK handles transport and argument validation.

## Shared worker architecture

The hosted ChatGPT path runs `tonk-worker-host`, built from the ordinary
`tonk-worker` router with filesystem storage. `worker-hosted-server.mjs` owns
OAuth, principal isolation and short-lived space sessions. Each principal uses
one live worker; the frontend and MCP agent read and mutate that same replica.

The hosted tool surface is `tonk_list_spaces`, `tonk_guide`, `tonk_query`,
`tonk_evaluate`, `tonk_space_info`, and `tonk_open_space`. Space operations require
an exact subject returned by `tonk_list_spaces`. Query and evaluate forward notation unchanged to the ordinary
worker evaluator; commands, installed views and libraries retain their normal
semantics. The desktop adapter consumes the same canonical `tonk_evaluate`
definition and routes it to its existing worker behind the write capability gate.

`worker-space.html` hosts the stock guest runtime in an opaque iframe. Its normal
HTTP requests and SSE subscriptions use `/space-api/` directly with a short-lived
session supplied only in tool `_meta`. Renderer traffic does not consume MCP tool
calls. The session selects one space/main branch and cannot reach account APIs.
There are no notebook-specific editing controls or command translations.

Build and test the isolated native frontend slice from the repository root:

```sh
cargo build -p tonk-worker --features native-host --bin tonk-worker-host
TONK_WORKER_BINARY="$PWD/target/debug/tonk-worker-host" node mcp/fixtures/worker-preview.mjs
```

Visit `http://127.0.0.1:8796`. The fixture uses eight loopback ports because browsers
limit HTTP/1 origins to six simultaneous streams; deployed HTTPS uses HTTP/2.
Typing `agent` in the fixture terminal changes its synthetic notebook through the
general MCP evaluator; the open frontend should update without refreshing.

Authorization seeds are checkpointed while closed. Live worker replicas stay open
and publish ordinary writes before returning success. A failed publication is an
uncertain write and is never automatically replayed. Live caches are not copied
while open. Issued OAuth tokens and browser sessions recover from a private
credential checkpoint. Incomplete approval ceremonies require a fresh attempt. The development flow has been exercised across a container replacement;
longer idle-period and failure-recovery coverage remains a release gate.

See [Cloudflare deployment](cloudflare/README.md). The earlier restricted CLI and
notebook adapters below remain as compatibility fixtures; the full-worker hosted
entrypoint does not expose them.

## Legacy restricted runtime and fixtures

- `tools.json`: shared desktop/core tool catalog, including host-specific tools.
- `core.mjs`: capability-filtered MCP registration and result/error envelopes.
- `native.mjs`: private child process running the monorepo's native Tonk runtime.
- `http.mjs`: SDK HTTP handler with a host-supplied backend resolver.
- `oauth.mjs`, `oauth-native.mjs`: bounded authorization-code/PKCE handshake
  through Tonk's browser approval and isolated native account identities.
- `authenticated-http.mjs`: verifies OAuth on each request before resolving a
  tenant's backend; independent SDK handlers prevent session-ID crossover.
- `tenant-backends.mjs`: checks the OAuth/native identity match and opens the
  host-selected account space, with one pinned child per tenant.
- `ui.mjs`, `ui/query.html`: optional MCP Apps query table with read-only refresh.
- `desktop.mjs`, `server.mjs`: existing desktop loopback adapter over stdio.

The native backend supports `tonk_space_info`, `tonk_query`, `tonk_preview`, and
`tonk_apply`, plus `tonk_install_library` for first installs of the bundled prose
and notebook components. Omit `expectedRevision` for a manifest preview; pass
that revision to install. Installation records complete library provenance and
publishes it atomically with the library. Existing versions and untracked models
are not upgraded or overwritten. The hosted adapter pushes committed installs
once; previews and already-installed results do not push. HTTP adds `tonk_show_query` and `tonk_show_notebook`. The model can define and populate a
reading-list concept through preview/apply, then display it. The query view is a table with direct notebook links. The notebook view reuses
Tonk’s prose renderer for ordered text, with read-only Refresh and Open in Tonk.
It reports unplaced blocks and refuses incomplete placed content. Live query
cells remain source; this is not the full interactive notebook runtime.

`tonk_open_space` is the real-frontend development slice. It resolves an installed
space route and runs Tonk's guest WASM, `tonk-display`, and stored view templates.
The guest is an opaque iframe, initially read-only, served from the MCP Worker's public
`space-guest.html`; its queries go through app-only `tonk_ui_read` and the selected
native replica. No browser sign-in, desktop process, or credentials enter the
frame. Root `/` follows the space's chosen home view, which may be blank.
Use `/notebook/<entity>` to open a notebook directly. Choose **Edit text** to edit
existing prose blocks. The stock notebook also derives its title from the first
heading. App-only `tonk_ui_begin_edit` / `tonk_ui_edit_notebook` adapt only these
existing-text/title commands to canonical preview/apply with a revision check
and one hosted push. Wait for **Saved and synced** before Refresh. Failed or
uncertain saves require read-back, never automatic retries. Block insertion,
deletion, code editing and profile navigation remain unsupported. Refresh loads
a new guest document and returns to read-only mode. Reads are initial snapshots;
this slice does not provide continuous remote subscriptions.

Build public assets with `node scripts/build-space-assets.mjs`. The script pins
production guest asset filenames and uses this checkout's portal/editor assets.
When production has retired those filenames, pass `TONK_GUEST_PACK=/path/to/space-runtime.json`
from a previously verified deployment with the same manifest. The builder rejects
HTML fallbacks and invalid Wasm headers before writing assets.
Upload ignored `public/` via the Worker assets binding. For a disposable real
renderer test, run the following from `mcp/`, then visit `http://127.0.0.1:8795`:

```sh
TONK_MCP_RUNTIME=../target/debug/tonk-mcp-runtime node fixtures/space-preview.mjs
```
This fixture never connects to the user's space.

The development runtime has real on-disk Tonk storage and opens one space with an
isolated software identity. A separate private account mode accepts Tonk browser
grants into a tenant-specific identity. The OAuth callback/token exchange is
implemented and tested locally. Private host controls can discover, pull, open
and explicitly push existing account spaces. The Cloudflare development
deployment is described in `cloudflare/README.md`. Account-wide creation and
a browser space picker remain outstanding.

## Run the standalone development service

```sh
cargo build --locked -p tonk-cli --bin tonk-mcp-runtime
cd mcp
npm ci --ignore-scripts
npm run build:ui
node dev-server.mjs ../target/debug/tonk-mcp-runtime /tmp/tonk-mcp-development
```

This serves MCP at `http://127.0.0.1:8787/mcp`, with no desktop process involved.
Keep the data directory to retain the same development space across restarts.
Only one runtime may own that directory at a time. The executable and data path
are host configuration, never tool arguments. No shell is used.

The development listener deliberately rejects non-loopback Host headers and
browser-origin requests. It is not a public service and is not ready to connect
from ChatGPT. Production hosting still needs:

1. Durable OAuth sessions, refresh/revocation lifecycle and operational limits.
2. User-facing space selection, runtime lifecycle and outbound-network policy.
3. Durable hosted storage and account-space creation.
4. HTTPS deployment and an actual ChatGPT UI test.

`createTonkHTTP(resolveBackend)` separates host authorization/site selection from
tools. The resolver must return a backend already authorized for that request;
the development singleton must not be reused as a multi-user production backend.
`tonk_cli::mcp_runtime::Runtime` accepts an explicitly opened `TonkSite`, so
production need not use the development binary's software identity.

## Private hosted account control

`startNativeAccount({ binary, dataDirectory })` in `native.mjs` launches
`tonk-mcp-runtime --account-data <directory>`. It returns `deviceDid`, `status()`,
`authorize(authorization, expectedAccount?)`, `listSpaces()`, `openSpace(subject)`,
and `close()`. Space controls are disabled unless the host explicitly passes
`enableSpaces: true` (native flag `--account-spaces`). It is **not an MCP backend**
and exposes no tool-call method before selection. The host chooses the
directory; no model-supplied path, account ID, or URL selects tenant storage.

The host can request Tonk's existing browser approval for `deviceDid`, using an
HTTPS callback. Before calling `authorize`, the OAuth service must bind
the callback to its pending browser session and authorization request, including
CSRF protection. `authorization` is the existing browser payload containing
`delegationHex`, `credentialId`, `attachmentId`, and optional legacy `remote`.
The native layer validates the delegation audience, signature, account-wide
shape, optional expected issuer, and attachment generation. Signed remote
metadata takes precedence over the callback's remote field.

Activation uses the CLI's canonical account journal. Returned identity/status
contains no grant bytes. An existing account cannot be overwritten by another
delivery. After a lost response, restart the process and inspect `status()`:
startup recovers an already-staged activation without repeating the browser
ceremony or contacting the remote service. Do not automatically replay a grant.

Account directories are private (0700 on Unix), exclusively locked, and marked
with their runtime mode. Existing development data cannot become a hosted
account directory, and account data cannot be opened in development mode.
Account mode creates no default space and enables no data tools after a grant
alone. With space controls enabled, `listSpaces()` hydrates the account directory
as needed. `openSpace(subject)` pulls the chosen DID through its retained
delegation chain and checks signed membership. Cached local registrations also
require a successful pull and membership check. A child cannot switch spaces
after selection; start another authorized child after closing it.

The selected result is an MCP backend with the shared query/preview/apply tools.
It additionally has host-only `pull()` and `push()` controls. Writes retain their
existing local-commit contract; upload is a separate operation and never repeats
the evaluation. This path uses explicit `SiteConfig` throughout account-space
discovery and mounting, without falling back to the server's global CLI profile.

`createTenantBackends({ binary, dataRoot, chooseSubject })` supplies the resolver
for `createAuthenticatedTonkHTTP`. `chooseSubject` must return the user's explicit
space selection, not a path or an automatic first-space choice. The resolver
checks the tenant directory, compares the native account/device identity with
the OAuth principal, and shares a child only for that same identity and subject.
Call its `close()` on shutdown. Its capacity is bounded, but idle eviction and
user-driven space switching are not implemented yet.

The network controls remain opt-in: production must install outbound-network
restrictions before enabling them. Account and mount metadata can name remote
services; this prototype does not yet enforce a hosted service allowlist or
prevent redirects to private network addresses. No public listener enables
these controls automatically.

## OAuth checkpoint

`createTonkOAuth` accepts an exact HTTPS `issuer` origin, one predefined public
`clientId`, exact `redirectUris`, and a `provisionAccount` adapter. Use
`nativeAccountProvisioner({ binary, dataRoot })` with an existing private root
directory. Each ceremony creates a host-selected tenant directory and native
identity. The adapter stops the child while the browser is approving, then
reopens it to validate and activate the returned grant.

The fetch handler serves standard protected-resource and authorization-server
metadata, `/oauth/authorize`, `/oauth/continue`, `/oauth/callback`, and
`/oauth/token`. It requires S256 PKCE, the exact `/mcp` resource and `tonk` scope.
An HttpOnly Secure cookie plus a random request ID bind browser delivery; the
callback also checks Origin. Its CSP-restricted page removes the grant fragment
from browser history before posting to the same origin. Redirects only target
the registered client; successful and redirected error responses include the
original state and exact issuer. Invalid client/redirect pairs never redirect.

Codes are single-use and expire after one minute. Opaque access tokens expire
after ten minutes and are stored by hash, bound to the authorized tenant. A
valid replay of a redeemed code revokes its credential family. Refresh tokens
rotate on every use; replay revokes the family and its embedded sessions. Access
can renew within a fixed eight-hour development connection window. `createAuthenticatedTonkHTTP`
checks the token before each request and passes only its trusted principal to
the host's `resolveBackend`. This prototype supports finite POST responses;
streaming subscriptions and direct cross-origin browser requests are disabled.

**This is a single-process spike, not the production authorization service.**
The shared-worker deployment persists issued credential families in private R2
before acknowledging issuance or rotation; incomplete approval ceremonies still
require a fresh attempt after restart. Other hosts remain in-memory unless they
wire the snapshot store. There is no DCR/CIMD, account revocation polling, idle-tenant
cleanup or public HTTP listener. Admission is bounded to 16 live ceremonies or
token/code families by default, but still needs deployment-level rate limits.
Native directories survive failures because the account may already be attached;
cleanup must coordinate with Tonk's service attachment lifecycle. The host must
resolve a properly hydrated, authorized space before exposing data tools. The
tenant resolver provides that private host boundary, but the user-facing picker
and deployment wiring are not implemented yet.

## Shared semantics

- A host pins the space; tool arguments cannot select arbitrary paths, URLs,
  repositories, branches or JavaScript.
- Query and preview accept bounded inline notation. No includes or YAML tags.
- Preview validates without committing and returns **current** matches/revision,
  not a proposed-state diff or rendered preview.
- Apply requires the exact preview revision, accepts durable changes only, and
  does not retry a write. An interrupted/lost response means an unknown outcome.
- Reads return complete bounded results or an explicit size error.
- An accepted write proves a local commit, not synchronization or rendered UI.
- UI resources are optional. Data tools remain useful without a UI host.

## Tonk Town consumption

Tonk Town's native definitions are generated from `tools.json`:

```sh
node mcp/scripts/sync-desktop.mjs /path/to/tonk-town
node mcp/scripts/sync-desktop.mjs --check /path/to/tonk-town
```

Commit the generated Swift file with the consumer change. Native builds need no
Node installation or sibling checkout. Existing Swift runtime handlers continue
to enforce authority and validation; the catalog defines the public contract.
The desktop sidebar's native proposal, CLI fallback, and rendered inspection
are optional host capabilities, not dependencies of the hosted service.

For external desktop agents, configure the monorepo's `server.mjs` with the
desktop's private connection-file path. Tonk Town's old launcher forwards here;
set `TONK_MCP_ROOT` in the MCP child environment when using a monorepo worktree.
The sync command also records this checkout in Tonk Town's ignored
`mcp/.canonical-root`, so local launchers work without an environment override.

## Verification

```sh
cargo test --locked -p tonk-cli --test mcp_runtime --test notation --bin tonk-mcp-runtime
cd mcp
TONK_MCP_RUNTIME="$PWD/../target/debug/tonk-mcp-runtime" npm test
```

Without `TONK_MCP_RUNTIME`, the real-native tests explicitly skip. With it, an SDK
HTTP client creates three reading-list entries through the real evaluator,
displays them, updates one, rejects a stale write, restarts the native process,
and reads the persisted update. These are local integration tests, not hosted or
ChatGPT proof. The separate stdio tests cover capability filtering, invalid
arguments, opaque revision preservation, and no replay after a lost write reply.
The Rust account tests sign real grants and verify tenant/audience isolation,
expected-account checks, durable activation, private-process delivery, and
restart. The Node account test checks the private adapter, process locking, and
mode separation. These fixtures do not prove a live passkey or OAuth ceremony.

After `npm ci` in `mcp`, explicitly run the cross-language OAuth test:

```sh
cargo test --locked -p tonk-cli --test mcp_runtime oauth_callback_accepts_real_native_grant -- --ignored
```

This test is ignored by default because it needs Node and installed MCP packages.
It signs a real disposable Tonk grant, delivers it through the OAuth callback,
exchanges the code, checks the token's tenant, and reopens native account state.
The separate OAuth tests cover invalid redirects, PKCE/resource/client binding,
cookie and origin checks, expiry, concurrent callbacks, code replay, and MCP
tenant isolation. None contacts a production Tonk account or ChatGPT.

The account-space integration suite uses a local access service and S3 fixture:

```sh
cargo test --locked -p tonk-cli --features integration-tests --test account_spaces
```

It verifies explicit tenant stores, rejection of cached spaces without signed
membership, and a real account-bound preview/apply/push read back from another
replica. The hosted runtime also reopens the selected space after that update.

Platform references checked 2026-10-07:
[architecture](https://developers.openai.com/plugins/concepts/plugins),
[UI](https://developers.openai.com/plugins/build/chatgpt-ui),
[authentication](https://developers.openai.com/plugins/build/auth).

The hosted frontend takes its application stylesheet and fonts from
`rust/tonk-ui/styles.css` and `rust/tonk-ui/assets/fonts`, alongside Web Awesome
CSS, matching the browser portal payload. Theme initialization is shared with
`rust/tonk-ui/index.html` through `assets/space-theme.js`. The generic guest mount
lives in `rust/tonk-portal/src/embed_bootstrap.js`; the MCP wrapper supplies the
route, FAB-hiding option and authenticated worker transport. Regenerate public
assets after changing these sources. Guest WASM is still pinned by the asset
builder; this does not imply the whole browser or desktop application is built
or deployed with the connector.

### Hosted authoring reference

`tonk_guide` returns the canonical CLI manuals without opening a space. Omit
`topic` for the catalog and MCP usage; use `notation`, `views`, and `events`
for native builds. Send notation documents, not CLI shell commands, to the
general evaluator. Regenerate bundled manuals after source edits with
`node mcp/scripts/build-authoring-guides.mjs`; tests check source parity.

## Acceptance evidence (2026-10-09)

The development connector was tested in ChatGPT with a real Tonk account.
The user confirmed that a normal app-building prompt in a fresh test space
produced a working task tracker after the canonical views/events guides were
updated. Earlier checks confirmed the stock embedded space, checkbox persistence,
and matching behavior in the browser runtime. This is user-observed acceptance,
not an automated browser or production-readiness claim.

The native authoring regression executes the guide's home and checkbox examples,
reads definitions and task values back, and checks site registration. The full
MCP/native suite had 66 passing tests with no skips at this checkpoint.

Remaining work before general availability:

- Idle-period, eight-hour expiry, revocation, multi-tab and interrupted-network
  recovery tests in ChatGPT; current authorizations have a fixed eight-hour limit.
- Bound live worker/disk retention as well as checkpoint retention; reconnects
  can still leave unused local replica directories until container replacement.
- Multi-user capacity and operational diagnostics. This is a single-container
  development deployment, not a production service configuration.
- Main-website passkey progress and latency are separate from this connector.
  The unfinished approval-page change is not included in this integration PR.

Run validation from the repository root after building both native binaries:

```sh
TONK_WORKER_BINARY="$PWD/target/debug/tonk-worker-host" \
TONK_MCP_RUNTIME="$PWD/target/debug/tonk-mcp-runtime" \
node --test --test-concurrency=1 mcp/*.test.mjs mcp/cloudflare/*.test.mjs
```
